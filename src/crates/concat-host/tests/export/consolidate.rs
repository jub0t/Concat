// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Merging selected clips into one rendered clip, end to end: the
//! selection rendered to a file, the file read back and checked against
//! what the timeline showed, then put in place of the clips and the whole
//! timeline exported again.

use super::*;

use concat_host::consolidate::Consolidation;

/// The clip that starts at `start` seconds.
fn starting_at(studio: &Studio, start: f64) -> String {
    studio
        .project()
        .active()
        .clips
        .iter()
        .find(|clip| (clip.start - start).abs() < 1e-6)
        .unwrap_or_else(|| panic!("a clip starts at {start}s"))
        .id
        .clone()
}

/// Renders `ids` the way the window does, into the project's media
/// folder, and reads the file back.
fn render(studio: &Studio, ids: &[String], name: &str) -> (Consolidation, PathBuf, Exported) {
    let plan = Consolidation::new(studio.project(), ids).expect("the selection can be merged");
    let root = Path::new(studio.session.path()).to_path_buf();
    let folder = root.join("media").join(format!("merged-{name}"));
    std::fs::create_dir_all(&folder).expect("makes the media folder");
    let output = folder.join("merged.mp4");
    let request = plan.request(studio.project(), &root, &output);
    export::run(&request, &AtomicBool::new(false), |_| {}).expect("the selection renders");
    let file = Exported::read(name, &output, (30, 1));
    (plan, output, file)
}

/// Puts the rendered file in place of the clips; returns the merged clip.
fn adopt(studio: &mut Studio, plan: &Consolidation, output: &Path) -> String {
    let mut item = media::probe(output.to_str().expect("utf-8"))
        .expect("the render probes")
        .to_new_media();
    item.name = "Merged clips".to_owned();
    let command = plan
        .replacement(studio.project(), item)
        .expect("nothing changed while rendering");
    studio
        .apply(command)
        .expect("the merged clip keeps the first clip's id")
}

/// Saves, reopens and exports the active timeline at its own rate,
/// comparing renders rather than the reopened document's raw seconds.
fn draw_whole(studio: &mut Studio, label: &str) -> Exported {
    studio.session.save(None).expect("saves");
    let reopened =
        Session::open(studio.session.path(), studio.session.settings()).expect("reopens");
    let output = studio.exports.join(format!("{label}.mp4"));
    let spec = ExportSpec {
        output: output.to_string_lossy().into_owned(),
        crf: 16,
        preset: "fast".into(),
        ..Default::default()
    };
    let request = export::request(&reopened, &spec, Vec::new());
    export::run(&request, &AtomicBool::new(false), |_| {}).expect("exports");
    Exported::read(label, &output, (30, 1))
}

fn near(got: impl IntoIterator<Item = u8>, want: impl IntoIterator<Item = u8>) -> bool {
    got.into_iter()
        .zip(want)
        .all(|(got, want)| got.abs_diff(want) <= 3)
}

#[test]
fn a_gap_inside_the_merge_becomes_dark_silent_frames_of_the_file() {
    let scratch = Scratch::new("merge-gap");
    let sources = Sources::make(scratch.path());
    let mut studio = Studio::new(scratch.path(), "Gap", video(WIDTH, HEIGHT, 30, 1));
    let media_id = studio.import(&sources.peek);
    studio.apply(Command::AddClipAtFirstFree {
        media_id,
        start: 0.0,
    });
    for time in [2.0, 4.0] {
        studio.apply(Command::SplitClips {
            clip_ids: studio.clip_ids(),
            time,
        });
    }
    let middle = starting_at(&studio, 2.0);
    studio.apply(Command::RemoveClips {
        clip_ids: vec![middle],
        ripple: false,
    });
    let ids = studio.clip_ids();
    let first = starting_at(&studio, 0.0);
    let span = studio.end();

    let (plan, output, file) = render(&studio, &ids, "gap");
    file.expect_length(6.0);
    file.expect_second(1.5, 1);
    file.expect_black(3.0);
    file.expect_second(4.5, 4);
    file.expect_second(5.5, 5);
    file.expect_tone(1.5);
    file.expect_quiet(3.0);
    file.expect_quiet(4.5);
    file.expect_tone(5.5);

    let merged = adopt(&mut studio, &plan, &output);
    assert_eq!(merged, first);
    assert_eq!(studio.clip_ids(), vec![first.clone()]);
    let clip = studio.clip(&first);
    assert_eq!(clip.start, 0.0);
    assert!(
        (clip.duration - span).abs() < 1e-9,
        "the merged clip runs the whole span, gap included: {}",
        clip.duration
    );

    let whole = studio.export("gap merged");
    whole.expect_length(6.0);
    whole.expect_second(1.5, 1);
    whole.expect_black(3.0);
    whole.expect_second(5.5, 5);
    whole.expect_tone(5.5);
}

#[test]
fn seconds_cut_out_of_the_recording_stay_out_of_the_merge() {
    let scratch = Scratch::new("merge-cuts");
    let sources = Sources::make(scratch.path());
    let mut studio = Studio::new(scratch.path(), "Cuts", video(WIDTH, HEIGHT, 30, 1));
    let media_id = studio.import(&sources.peek);
    studio.apply(Command::AddClipAtFirstFree {
        media_id,
        start: 0.0,
    });
    for time in [1.0, 3.0] {
        studio.apply(Command::SplitClips {
            clip_ids: studio.clip_ids(),
            time,
        });
    }
    let pause = starting_at(&studio, 1.0);
    studio.apply(Command::RemoveClips {
        clip_ids: vec![pause],
        ripple: true,
    });
    let ids = studio.clip_ids();
    assert_eq!(ids.len(), 2);

    // A render stopped before it starts leaves nothing behind.
    let plan = Consolidation::new(studio.project(), &ids).expect("mergeable");
    let stopped = scratch.path().join("stopped.mp4");
    let request = plan.request(studio.project(), Path::new(studio.session.path()), &stopped);
    assert!(export::run(&request, &AtomicBool::new(true), |_| {}).is_err());
    assert!(!stopped.exists(), "a cancelled render is not a file");

    let (plan, output, file) = render(&studio, &ids, "cuts");
    file.expect_length(4.0);
    file.expect_second(0.5, 0);
    file.expect_second(1.5, 3);
    file.expect_second(3.5, 5);
    file.expect_quiet(0.5);
    file.expect_tone(1.5);
    file.expect_tone(3.5);

    adopt(&mut studio, &plan, &output);
    let whole = studio.export("cuts merged");
    whole.expect_length(4.0);
    whole.expect_second(1.5, 3);
    whole.expect_second(2.5, 4);
    whole.expect_quiet(2.5);
}

#[test]
fn a_hidden_muted_lane_is_rendered_whole_and_stays_hidden_and_muted() {
    let scratch = Scratch::new("merge-hidden");
    let sources = Sources::make(scratch.path());
    let mut studio = Studio::new(scratch.path(), "Hidden", video(WIDTH, HEIGHT, 30, 1));
    let media_id = studio.import(&sources.peek);
    let id = studio
        .apply(Command::AddClipAtFirstFree {
            media_id,
            start: 0.0,
        })
        .expect("a clip");
    let lane = studio.clip(&id).track_id.clone();
    studio.apply(Command::SplitClips {
        clip_ids: studio.clip_ids(),
        time: 3.0,
    });
    for (flag, value) in [(TrackFlag::Visible, false), (TrackFlag::Muted, true)] {
        studio.apply(Command::SetTrackFlag {
            track_id: lane.clone(),
            flag,
            value,
        });
    }
    let ids = studio.clip_ids();

    let (plan, output, file) = render(&studio, &ids, "hidden");
    file.expect_second(1.5, 1);
    file.expect_second(4.5, 4);
    file.expect_tone(1.5);

    adopt(&mut studio, &plan, &output);
    let track = studio
        .project()
        .active()
        .tracks
        .iter()
        .find(|track| track.id == lane)
        .expect("the lane")
        .clone();
    assert!(
        !track.visible && track.muted,
        "the lane's switches are left as they were"
    );
    let whole = studio.export("hidden merged");
    whole.expect_black(1.5);
}

#[test]
fn a_speed_change_is_drawn_into_the_file_and_the_merged_clip_plays_at_normal_speed() {
    let scratch = Scratch::new("merge-speed");
    let sources = Sources::make(scratch.path());
    let mut studio = Studio::new(scratch.path(), "Speed", video(WIDTH, HEIGHT, 30, 1));
    let media_id = studio.import(&sources.peek);
    let fast = studio
        .apply(Command::AddClipAtFirstFree {
            media_id: media_id.clone(),
            start: 0.0,
        })
        .expect("a clip");
    studio.apply(Command::SetClipSpeed {
        clip_id: fast.clone(),
        speed: 2.0,
        ripple: false,
    });
    assert!(
        (studio.clip(&fast).duration - 3.0).abs() < 1e-6,
        "six seconds at double speed"
    );
    let after_fast = studio.end();
    let normal = studio
        .apply(Command::AddClipAtFirstFree {
            media_id,
            start: after_fast,
        })
        .expect("a clip");
    assert_eq!(
        studio.clip(&fast).track_id,
        studio.clip(&normal).track_id,
        "both on one lane"
    );

    let (plan, output, file) = render(&studio, &[fast.clone(), normal], "speed");
    file.expect_length(9.0);
    file.expect_second(0.75, 1);
    file.expect_second(2.25, 4);
    file.expect_second(3.5, 0);
    file.expect_second(8.5, 5);

    adopt(&mut studio, &plan, &output);
    let merged = studio.clip(&fast);
    assert_eq!(merged.speed, 1.0);
    assert!((merged.duration - 9.0).abs() < 1e-6);
    let whole = studio.export("speed merged");
    whole.expect_second(2.25, 4);
    whole.expect_second(8.5, 5);
}

#[test]
fn a_span_ending_inside_a_frame_is_still_covered_by_the_file() {
    let scratch = Scratch::new("merge-fraction");
    let source = scratch.path().join("plain.mp4");
    picture(&source, FrameRate::THIRTY, 3, VideoCodec::H264);
    let mut studio = Studio::new(scratch.path(), "Fraction", video(WIDTH, HEIGHT, 30, 1));
    let media_id = studio.import(&source);
    let id = studio
        .apply(Command::AddClipAtFirstFree {
            media_id,
            start: 0.0,
        })
        .expect("a clip");
    studio.apply(Command::TrimClip {
        clip_id: id,
        edge: TrimEdge::End,
        delta: -1.99,
        ripple: false,
    });
    studio.apply(Command::SplitClips {
        clip_ids: studio.clip_ids(),
        time: 0.5,
    });
    let ids = studio.clip_ids();
    let span = studio.end();
    assert!((span - 1.01).abs() < 1e-6);

    let (plan, output, file) = render(&studio, &ids, "fraction");
    assert!(
        file.frames.len() >= 31,
        "30.3 frames need 31, got {}",
        file.frames.len()
    );
    let merged = adopt(&mut studio, &plan, &output);
    assert!(
        (studio.clip(&merged).duration - span).abs() < 1e-9,
        "the merged clip is the span exactly, not the file"
    );
}

#[test]
fn an_effect_that_moves_with_time_keeps_its_timing_through_a_merge() {
    // A pulsing bloom is drawn twice over a wide blur, too slow on a
    // software adapter; there it runs only when CONCAT_REQUIRE_GPU asks.
    let software = export::WgpuCompositor::new().is_some_and(|gpu| gpu.on_software_adapter());
    if software && std::env::var_os("CONCAT_REQUIRE_GPU").is_none() {
        eprintln!("a software adapter only, and CONCAT_REQUIRE_GPU is not set; skipping");
        return;
    }
    let scratch = Scratch::new("merge-pulse");
    let source = scratch.path().join("grey.mp4");
    solid(&source, [200, 200, 200]);
    let mut studio = Studio::new(scratch.path(), "Pulse", video(WIDTH, HEIGHT, 30, 1));
    let media_id = studio.import(&source);
    let id = studio
        .apply(Command::AddClipAtFirstFree {
            media_id,
            start: 3.0,
        })
        .expect("a clip");
    studio.apply(Command::UpdateClip {
        clip_id: id,
        patch: ClipPatch {
            video_effects: Some(vec![AppliedFilter {
                id: "concat.bloom-pulse".into(),
                params: [("amount".into(), 90.0), ("speed".into(), 2.0)]
                    .into_iter()
                    .collect(),
                enabled: true,
                keys: Default::default(),
            }]),
            ..Default::default()
        },
    });
    studio.apply(Command::SplitClips {
        clip_ids: studio.clip_ids(),
        time: 4.0,
    });
    let original = studio.export("pulse before");
    let ids = studio.clip_ids();
    let (_plan, _output, file) = render(&studio, &ids, "pulse");
    for at in [0.1, 0.4, 0.7, 1.1, 1.6, 2.3] {
        let want = original.colour_at(3.0 + at);
        let got = file.colour_at(at);
        assert!(
            near(got, want),
            "at {at}s into the merge the pulse is {got:?}, the timeline showed {want:?}"
        );
    }
}

#[test]
fn after_a_merge_at_a_fractional_start_every_frame_exports_as_before() {
    let scratch = Scratch::new("merge-grid");
    let source = scratch.path().join("count.mp4");
    picture(&source, FrameRate::THIRTY, 3, VideoCodec::H264);
    let mut studio = Studio::new(scratch.path(), "Grid", video(WIDTH, HEIGHT, 30, 1));
    let media_id = studio.import(&source);
    studio.apply(Command::AddClipAtFirstFree {
        media_id,
        start: 2.02,
    });
    studio.apply(Command::SplitClips {
        clip_ids: studio.clip_ids(),
        time: 3.5,
    });
    let before = draw_whole(&mut studio, "grid before");
    let ids = studio.clip_ids();
    let (plan, output, _file) = render(&studio, &ids, "grid");
    adopt(&mut studio, &plan, &output);
    let after = draw_whole(&mut studio, "grid after");

    assert_eq!(after.frames.len(), before.frames.len());
    for (index, (got, want)) in after.frames.iter().zip(&before.frames).enumerate() {
        let got = got.pixel(WIDTH / 2, HEIGHT / 2).expect("a pixel");
        let want = want.pixel(WIDTH / 2, HEIGHT / 2).expect("a pixel");
        assert!(
            near(got, want),
            "frame {index}: {got:?} after merging, {want:?} before"
        );
    }
}

/// The bits of a counting frame's number, and the one lit block that says
/// it is footage at all: a row of eight blocks across the middle of the
/// frame, the first always lit, the next six the number's bits, lowest
/// first.
const COUNT_BLOCKS: u32 = 8;
const COUNT_BITS: u32 = 6;
/// A lit block and an unlit one: two hundred levels apart, so a lossy
/// encoder that moves a level by tens still reads each bit the same.
const LIT: u8 = 230;
const UNLIT: u8 = 30;

/// Forty-eight frames at 30 fps, each showing its own number in blocks
/// (see [`COUNT_BLOCKS`]). A frame is told by its number, not by a shade a
/// step from its neighbours': the CPU encoders the CI runners write with
/// move a flat colour by more than such a step, and a frame read back
/// eleven levels off looked like one three frames earlier.
fn counting(path: &Path) {
    let options = EncodeOptions {
        codec: VideoCodec::H264,
        preset: "ultrafast".to_owned(),
        crf: 8,
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
    let mut frame = Frame::black(WIDTH, HEIGHT);
    let block = WIDTH / COUNT_BLOCKS;
    for number in 0..48u32 {
        frame.fill([UNLIT, UNLIT, UNLIT, 255]);
        let stride = WIDTH as usize * 4;
        for slot in 0..COUNT_BLOCKS {
            let lit =
                slot == 0 || (1..=COUNT_BITS).contains(&slot) && number >> (slot - 1) & 1 == 1;
            if !lit {
                continue;
            }
            for y in 0..HEIGHT as usize {
                for x in (slot * block) as usize..((slot + 1) * block) as usize {
                    let at = y * stride + x * 4;
                    frame.pixels_mut()[at..at + 4].copy_from_slice(&[LIT, LIT, LIT, 255]);
                }
            }
        }
        encoder.write_frame(&frame).expect("writes a frame");
    }
    encoder.finish().expect("finishes the count");
}

/// The number a read-back frame of [`counting`] shows, or None where it
/// shows no footage - a gap. Each block is read at its centre against the
/// level half-way between lit and unlit.
fn number_of(frame: &Frame) -> Option<u32> {
    let block = frame.width() / COUNT_BLOCKS;
    let lit = |slot: u32| {
        let [r, g, b, _] = frame
            .pixel(slot * block + block / 2, frame.height() / 2)
            .expect("inside the frame");
        let level = (u32::from(r) + u32::from(g) + u32::from(b)) / 3;
        level > (u32::from(LIT) + u32::from(UNLIT)) / 2
    };
    lit(0).then(|| {
        (1..=COUNT_BITS)
            .filter(|&slot| lit(slot))
            .map(|slot| 1 << (slot - 1))
            .sum()
    })
}

/// #756: two halves of a clip split between frames could not be merged.
/// Pieces cut between frames, at a start off the grid, with a gap left
/// between two of them, merge at every rate into a clip that draws every
/// frame the pieces drew - none lost, none twice, the gap still dark -
/// and the clip after them does not move.
#[test]
fn pieces_cut_between_frames_merge_into_the_frames_they_covered() {
    let scratch = Scratch::new("merge-between-frames");
    let source = scratch.path().join("count.mp4");
    counting(&source);
    for (num, den) in [(30, 1), (24, 1), (30000, 1001), (60, 1)] {
        let label = format!("between frames at {num}/{den}");
        let mut studio = Studio::new(
            scratch.path(),
            &format!("Between-{num}"),
            video(WIDTH, HEIGHT, num, den),
        );
        let media_id = studio.import(&source);
        let track_id = studio.project().active().tracks[0].id.clone();
        let place = |studio: &mut Studio, start: f64| {
            studio
                .apply(Command::AddClip {
                    media_id: media_id.clone(),
                    track_id: track_id.clone(),
                    start,
                    ripple: false,
                })
                .expect("a clip")
        };
        let whole = place(&mut studio, 0.013);
        for time in [0.4371, 0.9123] {
            studio.apply(Command::SplitClips {
                clip_ids: vec![studio.clip_ids().last().expect("a clip").clone()],
                time,
            });
        }
        let gap = starting_at(&studio, 0.4371);
        studio.apply(Command::RemoveClips {
            clip_ids: vec![gap],
            ripple: false,
        });
        let selected = studio.clip_ids();
        assert_eq!(selected.len(), 2, "{label}: two pieces with a gap between");
        let end = studio.end();
        let after = place(&mut studio, end);
        let after_before = studio.clip(&after).clone();

        let draw = |studio: &mut Studio, name: &str| {
            studio.session.save(None).expect("saves");
            let reopened =
                Session::open(studio.session.path(), studio.session.settings()).expect("reopens");
            let output = studio.exports.join(format!("{name}.mp4"));
            let spec = ExportSpec {
                output: output.to_string_lossy().into_owned(),
                crf: 8,
                preset: "fast".into(),
                ..Default::default()
            };
            let request = export::request(&reopened, &spec, Vec::new());
            export::run(&request, &AtomicBool::new(false), |_| {}).expect("exports");
            Exported::read(name, &output, (num, den))
        };
        let before = draw(&mut studio, &format!("between-{num}-before"));

        let plan = Consolidation::new(studio.project(), &selected)
            .unwrap_or_else(|error| panic!("{label}: cannot merge: {error}"));
        let root = Path::new(studio.session.path()).to_path_buf();
        let folder = root.join("media").join(format!("merged-between-{num}"));
        std::fs::create_dir_all(&folder).expect("makes the media folder");
        let output = folder.join("merged.mp4");
        let request = plan.request(studio.project(), &root, &output);
        export::run(&request, &AtomicBool::new(false), |_| {}).expect("the selection renders");
        let merged = adopt(&mut studio, &plan, &output);
        assert_eq!(merged, whole, "{label}: the first piece's id is kept");
        assert_eq!(
            studio.clip(&after),
            &after_before,
            "{label}: the clip after the merge does not move"
        );
        let after_merge = draw(&mut studio, &format!("between-{num}-after"));

        assert_eq!(
            after_merge.frames.len(),
            before.frames.len(),
            "{label}: the timeline is as long"
        );
        let mut footage = 0;
        let mut gap_frames = 0;
        for (index, (got, want)) in after_merge.frames.iter().zip(&before.frames).enumerate() {
            let (got, want) = (number_of(got), number_of(want));
            footage += usize::from(want.is_some());
            gap_frames += usize::from(want.is_none());
            assert_eq!(
                got, want,
                "{label}: frame {index} shows source frame {got:?} after merging, {want:?} before"
            );
        }
        assert!(gap_frames > 0 && footage > 0, "{label}: a gap and footage");
    }
}

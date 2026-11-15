// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Merging selected clips through a render, without rendering: what is
//! captured from the selection, what the render is asked to draw and in
//! which format, and when the finished file may still replace the clips.

use std::path::Path;
use std::sync::Arc;

use concat_project::commands::{NewMedia, TrackFlag};
use concat_project::model::{Clip, ClipKind, ColorSpace, MediaKind, Project, VideoSettings};
use concat_project::{Command, DocumentSettings, Editor, consolidation_clips, to_document};

use crate::consolidate::Consolidation;
use crate::export::VideoCodec;

const CHANGED: &str = "The selected clips changed while merging. Please try again.";
const NO_MEDIA: &str = "A selected clip's media is missing.";
const CLOSE: f64 = 1e-9;

fn footage(path: &str) -> NewMedia {
    NewMedia {
        path: path.to_owned(),
        name: path.rsplit('/').next().unwrap_or(path).to_owned(),
        duration: Some(10.0),
        kind: MediaKind::Video,
        width: Some(1280),
        height: Some(720),
        frame_rate: Some(25.0),
        frame_rate_fraction: Some("25/1".to_owned()),
        video_codec: Some("h264".to_owned()),
        audio_codec: Some("aac".to_owned()),
        has_audio: true,
        audio_tracks: Vec::new(),
        origin: None,
        color_space: Default::default(),
    }
}

/// An editor holding a copy of `project`.
fn editor_of(project: &Project) -> Editor {
    let settings = DocumentSettings {
        name: "Merging".to_owned(),
        width: 1280,
        height: 720,
        rate_num: 25,
        rate_den: 1,
        canvas: [10, 20, 30, 255],
    };
    Editor::from_document(&to_document(&settings, project)).expect("the document reads")
}

fn finished(seconds: f64) -> NewMedia {
    let mut item = footage("/project/media/merged-1/merged.mp4");
    item.duration = Some(seconds);
    item.name = "Merged clips".to_owned();
    item
}

fn timeline_video(rate_num: i64, color_space: ColorSpace) -> VideoSettings {
    VideoSettings {
        width: 1280,
        height: 720,
        rate_num,
        rate_den: 1,
        color_space,
        canvas: [10, 20, 30, 255],
    }
}

/// A project with two files and, on the second lane, two clips to merge
/// (2 s to 12 s, and 14 s to 20 s from 4 s into the second file), a clip
/// after them on that lane, and one on the lane above.
struct Setup {
    project: Project,
    first: String,
    second: String,
    after: String,
    above: String,
}

fn setup(rate_num: i64, color_space: ColorSpace) -> Setup {
    let mut editor = Editor::with_video(timeline_video(rate_num, color_space));
    let import = |editor: &mut Editor, path: &str| {
        editor
            .apply(Command::AddMedia {
                item: footage(path),
            })
            .expect("imports")
            .created_id
            .expect("a media id")
    };
    let talk = import(&mut editor, "/footage/talk.mp4");
    let broll = import(&mut editor, "/footage/broll.mp4");
    let lanes: Vec<String> = editor
        .project()
        .active()
        .tracks
        .iter()
        .map(|track| track.id.clone())
        .collect();
    let place = |editor: &mut Editor, media: &str, lane: usize, start: f64| {
        editor
            .apply(Command::AddClip {
                media_id: media.to_owned(),
                track_id: lanes[lane].clone(),
                start,
                ripple: false,
            })
            .expect("places")
            .created_id
            .expect("a clip id")
    };
    let first = place(&mut editor, &talk, 1, 2.0);
    let second = place(&mut editor, &broll, 1, 14.0);
    let after = place(&mut editor, &talk, 1, 30.0);
    let above = place(&mut editor, &broll, 2, 0.0);
    let mut project = editor.project().clone();
    {
        let clip = project.active_mut().clip_mut(&first).expect("first");
        clip.volume = 0.3;
    }
    {
        let clip = project.active_mut().clip_mut(&second).expect("second");
        clip.source_start = 4.0;
        clip.duration = 6.0;
    }
    Setup {
        project,
        first,
        second,
        after,
        above,
    }
}

fn ids(setup: &Setup) -> Vec<String> {
    // Handed over in the wrong order on purpose.
    vec![setup.second.clone(), setup.first.clone()]
}

fn request_of(project: &Project, plan: &Consolidation) -> concat_export::ExportRequest {
    plan.request(
        project,
        Path::new("/project"),
        Path::new("/project/media/merged-1/merged.mp4"),
    )
}

fn is_marker(clip: &concat_export::ExportClip) -> bool {
    matches!(clip.kind, concat_export::ClipKind::Layer)
}

#[test]
fn a_selection_the_editor_refuses_is_refused_with_the_same_words() {
    let setup = setup(25, ColorSpace::Sdr);
    let lone = vec![setup.first.clone()];
    let expected = consolidation_clips(setup.project.active(), &lone)
        .expect_err("one clip cannot be merged")
        .to_string();
    let Err(error) = Consolidation::new(&setup.project, &lone) else {
        panic!("one clip was accepted");
    };
    assert_eq!(error, expected);

    let mut orphaned = setup.project.clone();
    orphaned.media.clear();
    let Err(error) = Consolidation::new(&orphaned, &ids(&setup)) else {
        panic!("clips with no file in the bin were accepted");
    };
    assert_eq!(error, NO_MEDIA);
}

#[test]
fn the_render_draws_the_selection_alone_moved_to_the_start() {
    let setup = setup(25, ColorSpace::Sdr);
    let plan = Consolidation::new(&setup.project, &ids(&setup)).expect("mergeable");
    assert_eq!(plan.track_id(), setup.project.active().tracks[1].id);
    let request = request_of(&setup.project, &plan);

    // 18 s at 25 fps is a whole number of frames: nothing is added.
    assert_eq!(request.clips.len(), 2, "only the two selected clips");
    let starts: Vec<f64> = request.clips.iter().map(|clip| clip.start).collect();
    assert!((starts[0] - 0.0).abs() < CLOSE && (starts[1] - 12.0).abs() < CLOSE);
    let ins: Vec<f64> = request.clips.iter().map(|clip| clip.source_start).collect();
    assert!((ins[0] - 0.0).abs() < CLOSE && (ins[1] - 4.0).abs() < CLOSE);
    assert!((request.clips[1].duration - 6.0).abs() < CLOSE);
    assert!(
        (request.clips[0].volume - 0.3).abs() < CLOSE,
        "the clip's level is kept"
    );
    assert!(
        request.clips.iter().all(|clip| clip.track == 1),
        "each clip keeps its lane"
    );
    assert!(
        (request.effect_time_offset - 2.0).abs() < CLOSE,
        "effects keep the timeline's clock"
    );
    assert!(setup.project.active().clip(&setup.after).is_some());
    assert!(setup.project.active().clip(&setup.above).is_some());
}

#[test]
fn the_render_is_at_the_timelines_frame_in_high_quality_h264() {
    let setup = setup(25, ColorSpace::Sdr);
    let plan = Consolidation::new(&setup.project, &ids(&setup)).expect("mergeable");
    let request = request_of(&setup.project, &plan);
    assert_eq!(request.output, "/project/media/merged-1/merged.mp4");
    assert_eq!((request.width, request.height), (1280, 720));
    assert_eq!((request.rate_num, request.rate_den), (25, 1));
    assert_eq!(request.canvas, [10, 20, 30, 255]);
    assert_eq!(request.color_space, ColorSpace::Sdr);
    assert_eq!(request.crf, 16);
    assert_eq!(request.preset, "fast");
    assert!(matches!(request.codec, VideoCodec::H264));
    assert!(!request.ten_bit);
    assert!(!request.hdr);
}

#[test]
fn an_hdr_timeline_renders_ten_bit_hevc_kept_as_hdr() {
    let setup = setup(25, ColorSpace::Hlg);
    let plan = Consolidation::new(&setup.project, &ids(&setup)).expect("mergeable");
    let request = request_of(&setup.project, &plan);
    assert!(matches!(request.codec, VideoCodec::Hevc));
    assert!(request.ten_bit);
    assert!(request.hdr);
    assert_eq!(request.color_space, ColorSpace::Hlg);
}

#[test]
fn hidden_and_muted_lanes_are_drawn_and_heard_in_the_render_only() {
    let setup = setup(25, ColorSpace::Sdr);
    let lane = setup.project.active().tracks[1].id.clone();
    let mut editor = editor_of(&setup.project);
    for flag in [TrackFlag::Visible, TrackFlag::Muted] {
        let value = matches!(flag, TrackFlag::Muted);
        editor
            .apply(Command::SetTrackFlag {
                track_id: lane.clone(),
                flag,
                value,
            })
            .expect("the lane changes");
    }
    let project = editor.project().clone();
    let plan = Consolidation::new(&project, &ids(&setup)).expect("mergeable");
    let request = request_of(&project, &plan);
    assert!(
        request.clips.iter().all(|clip| !clip.hidden && !clip.muted),
        "the file holds the picture and the sound; the lane decides later"
    );
    assert_eq!(
        &project,
        editor.project(),
        "the project itself is not touched"
    );
    let kept = project.active().track(&lane).expect("the lane");
    assert!(!kept.visible && kept.muted);
}

#[test]
fn a_span_that_ends_inside_a_frame_is_rendered_up_to_the_next_frame() {
    // At 30 fps: 0.5 s and 0.51 s are 1.01 s, 30.3 frames; the file must
    // hold 31 so the last fraction of a frame exists to be shown.
    let mut setup = setup(30, ColorSpace::Sdr);
    let (first, second) = (setup.first.clone(), setup.second.clone());
    {
        let timeline = setup.project.active_mut();
        let clip = timeline.clip_mut(&first).expect("first");
        clip.start = 0.0;
        clip.duration = 0.5;
        let clip = timeline.clip_mut(&second).expect("second");
        clip.start = 0.5;
        clip.duration = 0.51;
    }
    let plan = Consolidation::new(&setup.project, &ids(&setup)).expect("mergeable");
    let request = request_of(&setup.project, &plan);
    let markers: Vec<_> = request
        .clips
        .iter()
        .filter(|clip| is_marker(clip))
        .collect();
    assert_eq!(markers.len(), 1, "one stand-in sets the length");
    let marker = markers[0];
    assert!(marker.hidden && marker.muted);
    assert_eq!(marker.has_audio, Some(false));
    assert!(marker.start.abs() < CLOSE);
    assert!(
        (marker.duration - 31.0 / 30.0).abs() < 1e-6,
        "{}",
        marker.duration
    );
    assert_eq!(
        marker.track,
        setup.project.active().tracks.len(),
        "on a lane of its own"
    );
    let real: Vec<_> = request
        .clips
        .iter()
        .filter(|clip| !is_marker(clip))
        .collect();
    assert_eq!(real.len(), 2);
    assert!(
        (real[1].duration - 0.51).abs() < CLOSE,
        "the clips themselves are not stretched"
    );

    // A whole number of frames needs no stand-in.
    {
        let clip = setup
            .project
            .active_mut()
            .clip_mut(&second)
            .expect("second");
        clip.duration = 0.5;
    }
    let plan = Consolidation::new(&setup.project, &ids(&setup)).expect("mergeable");
    let request = request_of(&setup.project, &plan);
    assert!(!request.clips.iter().any(is_marker));
}

#[test]
fn the_finished_file_replaces_the_clips_in_timeline_order() {
    let setup = setup(25, ColorSpace::Sdr);
    let plan = Consolidation::new(&setup.project, &ids(&setup)).expect("mergeable");
    let command = plan
        .replacement(&setup.project, finished(18.0))
        .expect("nothing changed");
    let Command::ConsolidateClips { clip_ids, item } = command else {
        panic!("the replacement is not a merge");
    };
    assert_eq!(clip_ids, vec![setup.first.clone(), setup.second.clone()]);
    assert_eq!(item.path, "/project/media/merged-1/merged.mp4");

    let mut editor = editor_of(&setup.project);
    let made = editor
        .apply(Command::ConsolidateClips { clip_ids, item })
        .expect("the edit applies")
        .created_id;
    assert_eq!(made.as_deref(), Some(setup.first.as_str()));
}

#[test]
fn an_edit_to_what_was_rendered_refuses_the_file() {
    let setup = setup(25, ColorSpace::Sdr);
    let plan = Consolidation::new(&setup.project, &ids(&setup)).expect("mergeable");
    let refused = |project: &Project, what: &str| match plan.replacement(project, finished(18.0)) {
        Ok(_) => panic!("{what}: the stale render was accepted"),
        Err(error) => assert_eq!(error, CHANGED, "{what}"),
    };

    let mut quieter = setup.project.clone();
    quieter
        .active_mut()
        .clip_mut(&setup.second)
        .expect("second")
        .volume = 0.9;
    refused(&quieter, "a selected clip's level changed");

    let mut moved = setup.project.clone();
    moved
        .active_mut()
        .clip_mut(&setup.second)
        .expect("second")
        .start = 15.0;
    refused(&moved, "a selected clip moved");

    let mut removed = setup.project.clone();
    removed
        .active_mut()
        .clips
        .retain(|clip| clip.id != setup.second);
    refused(&removed, "a selected clip was deleted");

    let mut relinked = setup.project.clone();
    relinked.media[1].path = "/elsewhere/broll.mp4".to_owned();
    refused(&relinked, "a source was relinked");

    let mut faster = setup.project.clone();
    faster.active_mut().video.rate_num = 50;
    refused(&faster, "the timeline's rate changed");

    let mut elsewhere = setup.project.clone();
    let mut copy = (*elsewhere.timelines[0]).clone();
    copy.id = "TL-other".to_owned();
    elsewhere.timelines.push(Arc::new(copy));
    elsewhere.active_timeline_id = "TL-other".to_owned();
    refused(&elsewhere, "another timeline is open");

    let mut filled = setup.project.clone();
    let lane = filled.active().tracks[1].id.clone();
    let mut stray = Clip::blank("c-stray", lane, ClipKind::Video, "stray", 12.5, 1.0);
    stray.media_id = filled.media[0].id.clone();
    filled.active_mut().clips.push(Arc::new(stray));
    refused(&filled, "a clip was put into the gap");

    // Edits that do not touch the selection leave the render good.
    let mut unrelated = setup.project.clone();
    unrelated
        .active_mut()
        .clip_mut(&setup.above)
        .expect("above")
        .volume = 0.2;
    unrelated
        .active_mut()
        .clip_mut(&setup.after)
        .expect("after")
        .scale = 2.0;
    assert!(plan.replacement(&unrelated, finished(18.0)).is_ok());
}

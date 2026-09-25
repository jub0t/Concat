// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The edit itself, owned by the engine.
//!
//! The model ([`model`]), every operation as a serialisable
//! [`commands::Command`], the tolerant document reader and compatible writer
//! ([`doc`]), and the undo stack ([`editor::Editor`]) - all headless, all
//! testable without a window.
//!
//! Two rules hold throughout:
//!
//! 1. **The document format is frozen by the documents that exist.** Saved
//!    projects load forever, tolerance rules included, because users' work
//!    does not migrate on our schedule. Format changes go through
//!    `DOCUMENT_VERSION`.
//! 2. **f64 seconds and String ids are the document's terms**, kept until a
//!    version 2 decides otherwise on purpose.
//!
//! Deliberately a separate crate rather than part of `concat-core`: the
//! document model needs serde, and concat-core's zero-dependency rule is worth
//! more than the adjacency.

pub mod commands;
pub mod doc;
pub mod editor;
pub mod model;
pub mod speed;

pub use commands::{Command, CommandError, Outcome, why_not_merge};
pub use doc::{DOCUMENT_VERSION, DocumentSettings, document_version, from_document, to_document};
pub use editor::Editor;
pub use model::Project;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::commands::{ClipMove, ClipPatch, Command, NewMedia, TrackFlag, TrimEdge};
    use crate::doc::DocumentSettings;
    use crate::editor::Editor;
    use crate::model::{
        AudioTrack, ClipKind, MediaItem, MediaKind, MediaOrigin, Project, TextStyle,
    };

    fn media(path: &str, duration: f64, has_audio: bool) -> Command {
        Command::AddMedia {
            item: crate::commands::NewMedia {
                path: path.to_owned(),
                name: path.rsplit('/').next().unwrap_or(path).to_owned(),
                duration: Some(duration),
                kind: MediaKind::Video,
                width: Some(1920),
                height: Some(1080),
                frame_rate: Some(30.0),
                frame_rate_fraction: Some("30/1".to_owned()),
                video_codec: Some("h264".to_owned()),
                audio_codec: has_audio.then(|| "aac".to_owned()),
                has_audio,
                audio_tracks: Vec::new(),
                origin: None,
            },
        }
    }

    /// A recording that kept its desktop sound and microphone apart: two
    /// audio streams, the second named.
    fn two_track_media(path: &str) -> Command {
        let Command::AddMedia { mut item } = media(path, 10.0, true) else {
            unreachable!()
        };
        item.audio_tracks = vec![
            AudioTrack {
                index: 1,
                codec: "aac".to_owned(),
                channels: 2,
                sample_rate: 48_000,
                title: String::new(),
                language: String::new(),
            },
            AudioTrack {
                index: 2,
                codec: "aac".to_owned(),
                channels: 1,
                sample_rate: 48_000,
                title: "Mic/Aux".to_owned(),
                language: String::new(),
            },
        ];
        Command::AddMedia { item }
    }

    fn settings() -> DocumentSettings {
        DocumentSettings {
            name: "Test".to_owned(),
            width: 1920,
            height: 1080,
            rate_num: 30,
            rate_den: 1,
        }
    }

    /// Over JSON a null crop takes the crop off, as the patch documents:
    /// without `double_option` serde read null as "leave it", so no caller
    /// on the far side of the API could ever remove a crop.
    #[test]
    fn a_null_crop_on_the_wire_takes_the_crop_off() {
        let (mut editor, _, clip_id) = fixture();
        let patch = |crop: serde_json::Value| -> Command {
            serde_json::from_value(json!({
                "op": "updateClip",
                "clipId": clip_id,
                "patch": { "crop": crop }
            }))
            .expect("parses")
        };
        editor
            .apply(patch(
                json!({ "left": 0.2, "top": 0.0, "right": 0.0, "bottom": 0.1 }),
            ))
            .expect("crops");
        assert!(editor.project().active().clips[0].crop.is_some());
        let outcome = editor.apply(patch(json!(null))).expect("uncrops");
        assert!(outcome.applied, "null is take it off, not leave it");
        assert!(editor.project().active().clips[0].crop.is_none());

        let untouched: Command = serde_json::from_value(json!({
            "op": "updateClip",
            "clipId": clip_id,
            "patch": { "opacity": 0.5 }
        }))
        .expect("parses");
        match untouched {
            Command::UpdateClip { patch, .. } => assert_eq!(patch.crop, None),
            _ => panic!("not an update"),
        }
    }

    /// Editor with one media item and one clip at [0, 10) on track one.
    fn fixture() -> (Editor, String, String) {
        let mut editor = Editor::new();
        let media_id = editor
            .apply(media("/a.mp4", 10.0, true))
            .expect("adds")
            .created_id
            .expect("id");
        let track_id = editor.project().active().tracks[0].id.clone();
        let clip_id = editor
            .apply(Command::AddClip {
                media_id: media_id.clone(),
                track_id,
                start: 0.0,
                ripple: false,
            })
            .expect("adds")
            .created_id
            .expect("id");
        (editor, media_id, clip_id)
    }

    /// Clips of `media_id`, ten seconds each, at these starts on this
    /// track, in this order. The ids come back in the same order.
    fn lane(editor: &mut Editor, media_id: &str, track_id: &str, starts: &[f64]) -> Vec<String> {
        starts
            .iter()
            .map(|start| {
                editor
                    .apply(Command::AddClip {
                        media_id: media_id.to_owned(),
                        track_id: track_id.to_owned(),
                        start: *start,
                        ripple: false,
                    })
                    .expect("adds")
                    .created_id
                    .expect("id")
            })
            .collect()
    }

    fn start_of(editor: &Editor, clip_id: &str) -> f64 {
        editor
            .project()
            .active()
            .clip(clip_id)
            .expect("the clip is still there")
            .start
    }

    // ── ripple delete: https://github.com/jub0t/Concat/issues/106 ──

    #[test]
    fn ripple_delete_closes_the_gap_on_its_own_track_only() {
        let (mut editor, media_id, first) = fixture();
        let (video, sound) = {
            let tracks = &editor.project().active().tracks;
            (tracks[0].id.clone(), tracks[1].id.clone())
        };
        let rest = lane(&mut editor, &media_id, &video, &[10.0, 20.0]);
        let other = lane(&mut editor, &media_id, &sound, &[15.0]);

        let outcome = editor
            .apply(Command::RemoveClips {
                clip_ids: vec![rest[0].clone()],
                ripple: true,
            })
            .expect("removes");
        assert!(outcome.applied);
        assert!(editor.project().active().clip(&rest[0]).is_none());
        assert_eq!(start_of(&editor, &first), 0.0, "what was in front stays");
        assert_eq!(
            start_of(&editor, &rest[1]),
            10.0,
            "what was behind closes up"
        );
        assert_eq!(
            start_of(&editor, &other[0]),
            15.0,
            "another lane is none of this deletion's business"
        );
    }

    #[test]
    fn a_plain_delete_still_leaves_the_hole() {
        let (mut editor, media_id, _) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        let rest = lane(&mut editor, &media_id, &video, &[10.0, 20.0]);
        editor
            .apply(Command::RemoveClips {
                clip_ids: vec![rest[0].clone()],
                ripple: false,
            })
            .expect("removes");
        assert_eq!(start_of(&editor, &rest[1]), 20.0);
    }

    #[test]
    fn ripple_delete_of_several_clips_moves_each_survivor_by_what_was_before_it() {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        let rest = lane(&mut editor, &media_id, &video, &[10.0, 20.0, 30.0]);
        // The first and the third go: the second moves by one span, the
        // fourth by two.
        editor
            .apply(Command::RemoveClips {
                clip_ids: vec![first, rest[1].clone()],
                ripple: true,
            })
            .expect("removes");
        assert_eq!(start_of(&editor, &rest[0]), 0.0);
        assert_eq!(start_of(&editor, &rest[2]), 10.0);
        assert_eq!(editor.project().active().clips.len(), 2);
    }

    #[test]
    fn ripple_delete_counts_overlapping_spans_once() {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        // Two doomed clips sharing five seconds - [0, 10) and [5, 15) -
        // and a survivor at twenty. Their union is fifteen, so it lands at
        // five; their sum is twenty, which would send it to zero. Away
        // from zero on purpose: the floor there would hide the difference.
        let more = lane(&mut editor, &media_id, &video, &[5.0, 20.0]);
        editor
            .apply(Command::RemoveClips {
                clip_ids: vec![first, more[0].clone()],
                ripple: true,
            })
            .expect("removes");
        assert_eq!(start_of(&editor, &more[1]), 5.0);
    }

    #[test]
    fn ripple_delete_of_a_span_reaching_past_a_survivor_pulls_only_up_to_it() {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        // A doomed clip at [10, 20) with a survivor stacked inside its span
        // at fourteen: the survivor moves by the four seconds of span in
        // front of it, to ten, not by the whole ten, which would put it at
        // four - in front of where the doomed clip began.
        let more = lane(&mut editor, &media_id, &video, &[10.0, 14.0]);
        editor
            .apply(Command::RemoveClips {
                clip_ids: vec![more[0].clone()],
                ripple: true,
            })
            .expect("removes");
        assert_eq!(start_of(&editor, &first), 0.0);
        assert_eq!(start_of(&editor, &more[1]), 10.0);
    }

    #[test]
    fn ripple_delete_leaves_a_clip_stacked_at_the_same_start_alone() {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        let twin = lane(&mut editor, &media_id, &video, &[0.0]);
        editor
            .apply(Command::RemoveClips {
                clip_ids: vec![first],
                ripple: true,
            })
            .expect("removes");
        assert_eq!(
            start_of(&editor, &twin[0]),
            0.0,
            "nothing was in front of it"
        );
    }

    #[test]
    fn ripple_delete_of_unknown_ids_moves_nothing_and_records_nothing() {
        let (mut editor, media_id, _) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        let rest = lane(&mut editor, &media_id, &video, &[10.0]);
        let before_can_undo = editor.can_undo();
        let outcome = editor
            .apply(Command::RemoveClips {
                clip_ids: vec!["nobody".to_owned()],
                ripple: true,
            })
            .expect("tolerated");
        assert!(!outcome.applied);
        assert_eq!(start_of(&editor, &rest[0]), 10.0);
        assert_eq!(editor.can_undo(), before_can_undo);
    }

    #[test]
    fn ripple_delete_is_one_undo_step_that_puts_everything_back() {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        let rest = lane(&mut editor, &media_id, &video, &[10.0, 20.0]);
        editor
            .apply(Command::RemoveClips {
                clip_ids: vec![first.clone(), rest[0].clone()],
                ripple: true,
            })
            .expect("removes");
        assert_eq!(start_of(&editor, &rest[1]), 0.0);
        assert!(editor.undo());
        assert_eq!(start_of(&editor, &first), 0.0);
        assert_eq!(start_of(&editor, &rest[0]), 10.0);
        assert_eq!(start_of(&editor, &rest[1]), 20.0);
        assert!(editor.redo());
        assert_eq!(start_of(&editor, &rest[1]), 0.0);
    }

    #[test]
    fn ripple_delete_of_everything_on_a_lane_is_just_a_delete() {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        let rest = lane(&mut editor, &media_id, &video, &[10.0]);
        editor
            .apply(Command::RemoveClips {
                clip_ids: vec![first, rest[0].clone()],
                ripple: true,
            })
            .expect("removes");
        assert!(editor.project().active().clips.is_empty());
    }

    #[test]
    fn a_remove_clips_document_without_the_ripple_field_still_parses_as_a_plain_delete() {
        let command: Command = serde_json::from_value(json!({
            "op": "removeClips",
            "clipIds": ["c1"]
        }))
        .expect("the field is new; old callers do not send it");
        match command {
            Command::RemoveClips { ripple, .. } => assert!(!ripple),
            _ => panic!("not a remove"),
        }
    }

    // ── magnetic trim: https://github.com/jub0t/Concat/issues/106 ──

    fn trim(clip_id: &str, edge: TrimEdge, delta: f64, ripple: bool) -> Command {
        Command::TrimClip {
            clip_id: clip_id.to_owned(),
            edge,
            delta,
            ripple,
        }
    }

    #[test]
    fn magnetic_tail_trim_moves_the_lane_behind_it_both_ways() {
        let (mut editor, media_id, first) = fixture();
        let (video, sound) = {
            let tracks = &editor.project().active().tracks;
            (tracks[0].id.clone(), tracks[1].id.clone())
        };
        let behind = lane(&mut editor, &media_id, &video, &[10.0, 20.0]);
        let other = lane(&mut editor, &media_id, &sound, &[12.0]);

        editor
            .apply(trim(&first, TrimEdge::End, -2.0, true))
            .expect("shortens");
        assert_eq!(start_of(&editor, &behind[0]), 8.0, "closes up");
        assert_eq!(
            start_of(&editor, &behind[1]),
            18.0,
            "and the one behind that"
        );
        assert_eq!(start_of(&editor, &other[0]), 12.0, "another lane stays");

        editor
            .apply(trim(&first, TrimEdge::End, 3.0, true))
            .expect("lengthens");
        assert_eq!(start_of(&editor, &behind[0]), 11.0, "makes room");
        assert_eq!(start_of(&editor, &behind[1]), 21.0);
    }

    #[test]
    fn a_plain_trim_still_leaves_the_lane_alone() {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        let behind = lane(&mut editor, &media_id, &video, &[10.0]);
        editor
            .apply(trim(&first, TrimEdge::End, -2.0, false))
            .expect("shortens");
        assert_eq!(start_of(&editor, &behind[0]), 10.0);
    }

    #[test]
    fn magnetic_head_trim_keeps_the_clip_where_it_was_and_closes_behind_it() {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        let behind = lane(&mut editor, &media_id, &video, &[10.0]);
        editor
            .apply(trim(&first, TrimEdge::Start, 2.0, true))
            .expect("trims the head");
        let clip = editor
            .project()
            .active()
            .clip(&first)
            .expect("kept")
            .clone();
        assert_eq!(clip.start, 0.0, "the clip does not move");
        assert_eq!(clip.duration, 8.0);
        assert_eq!(clip.source_start, 2.0, "the in-point does");
        assert_eq!(start_of(&editor, &behind[0]), 8.0, "and the lane closes");
    }

    #[test]
    fn a_plain_head_trim_moves_the_head_and_nothing_else() {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        let behind = lane(&mut editor, &media_id, &video, &[10.0]);
        editor
            .apply(trim(&first, TrimEdge::Start, 2.0, false))
            .expect("trims the head");
        assert_eq!(start_of(&editor, &first), 2.0);
        assert_eq!(start_of(&editor, &behind[0]), 10.0);
    }

    #[test]
    fn magnetic_head_trim_can_reach_back_only_as_far_as_the_source_and_moves_nothing_when_it_cannot()
     {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        let behind = lane(&mut editor, &media_id, &video, &[10.0]);
        let before_can_undo = editor.can_undo();
        // In-point at zero: there is nothing before it to restore.
        let outcome = editor
            .apply(trim(&first, TrimEdge::Start, -3.0, true))
            .expect("tolerated");
        assert!(!outcome.applied);
        assert_eq!(start_of(&editor, &behind[0]), 10.0);
        assert_eq!(editor.can_undo(), before_can_undo, "a no-op is not a step");
    }

    #[test]
    fn magnetic_head_trim_restoring_source_pushes_the_lane_back() {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        let behind = lane(&mut editor, &media_id, &video, &[10.0]);
        editor
            .apply(trim(&first, TrimEdge::Start, 4.0, true))
            .expect("cuts four off the head");
        assert_eq!(start_of(&editor, &behind[0]), 6.0);
        // Ask for five back: only four exist, so four come back.
        editor
            .apply(trim(&first, TrimEdge::Start, -5.0, true))
            .expect("restores what there is");
        let clip = editor
            .project()
            .active()
            .clip(&first)
            .expect("kept")
            .clone();
        assert_eq!(
            clip.start, 0.0,
            "a magnetic clip at zero still does not move"
        );
        assert_eq!(clip.source_start, 0.0);
        assert_eq!(clip.duration, 10.0);
        assert_eq!(
            start_of(&editor, &behind[0]),
            10.0,
            "the lane went back by four"
        );
    }

    #[test]
    fn magnetic_trim_stops_at_the_minimum_and_the_lane_stays_touching() {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        let behind = lane(&mut editor, &media_id, &video, &[10.0]);
        editor
            .apply(trim(&first, TrimEdge::End, -100.0, true))
            .expect("clamps");
        let clip = editor
            .project()
            .active()
            .clip(&first)
            .expect("kept")
            .clone();
        assert!(
            clip.duration > 0.0 && clip.duration < 1.0,
            "floored at the minimum"
        );
        let gap = start_of(&editor, &behind[0]) - (clip.start + clip.duration);
        assert!(
            gap.abs() < 1e-9,
            "the lane moved by the clamped amount, not the asked one: gap {gap}"
        );
    }

    #[test]
    fn magnetic_trim_leaves_a_clip_in_front_of_the_edge_alone() {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        // A clip behind the trimmed one, and one stacked before its tail
        // on the same lane: a tail trim is about what is behind the tail.
        let more = lane(&mut editor, &media_id, &video, &[10.0, 3.0]);
        editor
            .apply(trim(&first, TrimEdge::End, -2.0, true))
            .expect("shortens");
        assert_eq!(start_of(&editor, &more[0]), 8.0);
        assert_eq!(
            start_of(&editor, &more[1]),
            3.0,
            "in front of the edge; not moved"
        );
    }

    #[test]
    fn magnetic_trim_is_one_undo_step_that_puts_the_lane_back() {
        let (mut editor, media_id, first) = fixture();
        let video = editor.project().active().tracks[0].id.clone();
        let behind = lane(&mut editor, &media_id, &video, &[10.0]);
        editor
            .apply(trim(&first, TrimEdge::Start, 2.0, true))
            .expect("trims");
        assert_eq!(start_of(&editor, &behind[0]), 8.0);
        assert!(editor.undo());
        assert_eq!(start_of(&editor, &behind[0]), 10.0);
        let clip = editor
            .project()
            .active()
            .clip(&first)
            .expect("kept")
            .clone();
        assert_eq!(
            (clip.start, clip.duration, clip.source_start),
            (0.0, 10.0, 0.0)
        );
    }

    #[test]
    fn a_trim_clip_document_without_the_ripple_field_still_parses_as_a_plain_trim() {
        let command: Command = serde_json::from_value(json!({
            "op": "trimClip",
            "clipId": "c1",
            "edge": "end",
            "delta": -1.0
        }))
        .expect("the field is new; old callers do not send it");
        match command {
            Command::TrimClip { ripple, .. } => assert!(!ripple),
            _ => panic!("not a trim"),
        }
    }

    /// A document from before the box had a height reads as no height,
    /// and a hand-edited one cannot make it negative or not a number.
    /// https://github.com/jub0t/Concat/issues/119
    #[test]
    fn a_text_style_without_a_box_height_reads_as_the_words_own() {
        let style: TextStyle =
            serde_json::from_value(json!({ "content": "Hello" })).expect("parses");
        assert_eq!(style.max_height, 0.0);
        let odd = TextStyle {
            max_height: -3.0,
            ..TextStyle::default()
        }
        .tidy();
        assert_eq!(odd.max_height, 0.0);
        let nan = TextStyle {
            max_height: f64::NAN,
            ..TextStyle::default()
        }
        .tidy();
        assert_eq!(nan.max_height, 0.0);
    }

    #[test]
    fn a_new_project_has_one_timeline_and_four_lanes() {
        let editor = Editor::new();
        assert_eq!(editor.project().timelines.len(), 1);
        assert_eq!(editor.project().active().tracks.len(), 4);
    }

    #[test]
    fn duplicate_media_paths_are_ignored() {
        let mut editor = Editor::new();
        editor.apply(media("/a.mp4", 10.0, true)).expect("adds");
        editor.apply(media("/a.mp4", 10.0, true)).expect("no-op");
        assert_eq!(editor.project().media.len(), 1);
    }

    #[test]
    fn replace_clip_media_can_move_the_in_point_to_the_copy() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::TrimClip {
                clip_id: clip_id.clone(),
                edge: TrimEdge::Start,
                delta: 2.0,
                ripple: false,
            })
            .expect("trims");
        let trimmed = editor.project().active().clip(&clip_id).expect("clip");
        assert_eq!((trimmed.source_start, trimmed.duration), (2.0, 8.0));
        let copy = NewMedia {
            path: "/project/cache/reverse-1-2000-8000.mp4".into(),
            name: "a.mp4 (reversed)".into(),
            duration: Some(8.0),
            kind: MediaKind::Video,
            width: Some(1920),
            height: Some(1080),
            frame_rate: Some(30.0),
            frame_rate_fraction: Some("30/1".into()),
            video_codec: Some("h264".into()),
            audio_codec: None,
            has_audio: false,
            audio_tracks: Vec::new(),
            origin: None,
        };
        let outcome = editor
            .apply(Command::ReplaceClipMedia {
                clip_id: clip_id.clone(),
                item: copy,
                source_start: Some(0.0),
            })
            .expect("replaces");
        assert!(outcome.applied);
        let clip = editor.project().active().clip(&clip_id).expect("clip");
        assert_eq!(clip.source_start, 0.0, "the copy starts at its own zero");
        assert_eq!(clip.duration, 8.0, "the length is kept");
    }

    #[test]
    fn replace_clip_media_points_the_clip_at_the_copy_and_keeps_the_original() {
        let (mut editor, media_id, clip_id) = fixture();
        let copy = NewMedia {
            path: "/project/cache/enhance-1-2x.mp4".into(),
            name: "a.mp4 (enhanced)".into(),
            duration: Some(10.0),
            kind: MediaKind::Video,
            width: Some(3840),
            height: Some(2160),
            frame_rate: Some(30.0),
            frame_rate_fraction: Some("30/1".into()),
            video_codec: Some("h264".into()),
            audio_codec: Some("aac".into()),
            has_audio: true,
            audio_tracks: Vec::new(),
            origin: None,
        };
        let outcome = editor
            .apply(Command::ReplaceClipMedia {
                clip_id: clip_id.clone(),
                item: copy.clone(),
                source_start: None,
            })
            .expect("replaces");
        assert!(outcome.applied);
        let copy_id = outcome.created_id.expect("the copy's id");
        assert_ne!(copy_id, media_id);
        let clip = editor.project().active().clip(&clip_id).expect("clip");
        assert_eq!(clip.media_id, copy_id);
        assert_eq!(clip.source_start, 0.0, "the in-point is kept");
        assert_eq!(
            editor.project().media.len(),
            2,
            "the original stays in the bin"
        );
        assert_eq!(
            editor.project().media_by_id(&copy_id).expect("copy").width,
            Some(3840)
        );

        // The same copy again changes nothing and mints nothing.
        let again = editor
            .apply(Command::ReplaceClipMedia {
                clip_id: clip_id.clone(),
                item: copy.clone(),
                source_start: None,
            })
            .expect("no-op");
        assert!(!again.applied);
        assert_eq!(editor.project().media.len(), 2);

        // A clip nobody has is a no-op that adds nothing to the bin.
        let nobody = editor
            .apply(Command::ReplaceClipMedia {
                clip_id: "c999".into(),
                item: NewMedia {
                    path: "/elsewhere.mp4".into(),
                    ..copy
                },
                source_start: None,
            })
            .expect("no-op");
        assert!(!nobody.applied);
        assert_eq!(editor.project().media.len(), 2);

        // A non-finite number in the copy's facts is refused, like an import's.
        let bad = NewMedia {
            duration: Some(f64::NAN),
            path: "/nan.mp4".into(),
            ..NewMedia {
                path: String::new(),
                name: String::new(),
                duration: None,
                kind: MediaKind::Video,
                width: None,
                height: None,
                frame_rate: None,
                frame_rate_fraction: None,
                video_codec: None,
                audio_codec: None,
                has_audio: false,
                audio_tracks: Vec::new(),
                origin: None,
            }
        };
        assert!(
            editor
                .apply(Command::ReplaceClipMedia {
                    clip_id: clip_id.clone(),
                    item: bad,
                    source_start: None,
                })
                .is_err()
        );

        // Undo puts the clip back on the original; the copy stays in the
        // bin, harmless, for redo.
        editor.undo();
        assert_eq!(
            editor
                .project()
                .active()
                .clip(&clip_id)
                .expect("clip")
                .media_id,
            media_id
        );
    }

    #[test]
    fn freeze_frame_holds_a_second_and_ripples_the_tail() {
        let (mut editor, media_id, clip_id) = fixture();
        let still = NewMedia {
            path: "/freeze.jpg".into(),
            name: "freeze.jpg".into(),
            duration: None,
            kind: MediaKind::Image,
            width: Some(1920),
            height: Some(1080),
            frame_rate: None,
            frame_rate_fraction: None,
            video_codec: None,
            audio_codec: None,
            has_audio: false,
            audio_tracks: Vec::new(),
            origin: None,
        };
        let freeze_id = editor
            .apply(Command::FreezeFrame {
                clip_id: clip_id.clone(),
                time: 4.0,
                duration: Some(1.0),
                still: Some(still),
            })
            .expect("freezes")
            .created_id
            .expect("freeze id");

        let clips = &editor.project().active().clips;
        assert_eq!(clips.len(), 3, "head + freeze + tail");
        let freeze = clips
            .iter()
            .find(|clip| clip.id == freeze_id)
            .expect("freeze");
        assert_eq!(freeze.kind, ClipKind::Image);
        assert_eq!(freeze.start, 4.0);
        assert_eq!(freeze.duration, 1.0);
        let tail = clips
            .iter()
            .find(|clip| clip.id != clip_id && clip.id != freeze_id)
            .expect("tail");
        assert_eq!(tail.start, 5.0, "tail ripples by the hold");
        assert_eq!(tail.source_start, 4.0);
        assert!(
            editor
                .project()
                .media
                .iter()
                .any(|item| item.id != media_id && item.path == "/freeze.jpg"),
            "still is imported"
        );
    }

    /// A freeze cuts the clip the way a split does, so a curved clip has to
    /// come out of it the way a split leaves one: both pieces at the curve's
    /// constant mean, meeting at the frozen source time.
    #[test]
    fn freeze_frame_on_a_curved_clip_keeps_the_pieces_continuous() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::SetClipSpeedCurve {
                clip_id: clip_id.clone(),
                curve: Some(vec![
                    crate::model::SpeedPoint {
                        at: 0.0,
                        speed: 0.5,
                    },
                    crate::model::SpeedPoint {
                        at: 1.0,
                        speed: 2.0,
                    },
                ]),
            })
            .expect("curves");
        let freeze_id = editor
            .apply(Command::FreezeFrame {
                clip_id: clip_id.clone(),
                time: 4.0,
                duration: Some(1.0),
                still: Some(NewMedia {
                    path: "/freeze.jpg".into(),
                    name: "freeze.jpg".into(),
                    duration: None,
                    kind: MediaKind::Image,
                    width: Some(1920),
                    height: Some(1080),
                    frame_rate: None,
                    frame_rate_fraction: None,
                    video_codec: None,
                    audio_codec: None,
                    has_audio: false,
                    audio_tracks: Vec::new(),
                    origin: None,
                }),
            })
            .expect("freezes")
            .created_id
            .expect("freeze id");

        let timeline = editor.project().active();
        let head = timeline.clip(&clip_id).expect("head");
        let tail = timeline
            .clips
            .iter()
            .find(|clip| clip.id != clip_id && clip.id != freeze_id)
            .expect("tail");
        for piece in [head, tail] {
            assert!(
                piece.speed_curve.is_none(),
                "a piece kept a curve its in-point was not computed for"
            );
        }
        assert_eq!(
            head.source_start + head.duration * head.speed,
            tail.source_start,
            "the tail picks up where the head ends"
        );
    }

    #[test]
    fn split_produces_source_continuous_halves_and_merge_rejoins_them() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::SplitClips {
                clip_ids: vec![clip_id.clone()],
                time: 4.0,
            })
            .expect("splits");

        let clips = &editor.project().active().clips;
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].duration, 4.0);
        assert_eq!(clips[1].start, 4.0);
        assert_eq!(clips[1].source_start, 4.0);

        let ids: Vec<String> = clips.iter().map(|clip| clip.id.clone()).collect();
        editor
            .apply(Command::MergeClips { clip_ids: ids })
            .expect("merges");
        let clips = &editor.project().active().clips;
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].duration, 10.0);
        assert_eq!(clips[0].id, clip_id, "the first piece keeps its identity");
    }

    #[test]
    fn a_head_trim_cannot_reach_before_the_source_begins() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::MoveClips {
                moves: vec![crate::commands::ClipMove {
                    clip_id: clip_id.clone(),
                    start: 5.0,
                    track_id: editor.project().active().clips[0].track_id.clone(),
                }],
            })
            .expect("moves");
        // Pulling the head four seconds left would want source -4: the trim
        // stops at the source's start, and the pixels do not slide.
        editor
            .apply(Command::TrimClip {
                clip_id: clip_id.clone(),
                edge: TrimEdge::Start,
                delta: -4.0,
                ripple: false,
            })
            .expect("trims");
        let clip = &editor.project().active().clips[0];
        assert_eq!(
            (clip.start, clip.duration, clip.source_start),
            (5.0, 10.0, 0.0)
        );
    }

    #[test]
    fn a_video_whose_detached_sound_was_deleted_can_be_unmuted() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::DetachAudio {
                clip_id: clip_id.clone(),
            })
            .expect("detaches");
        let sound: Vec<String> = editor
            .project()
            .active()
            .clips
            .iter()
            .filter(|clip| clip.id != clip_id)
            .map(|clip| clip.id.clone())
            .collect();
        assert_eq!(sound.len(), 1);
        editor
            .apply(Command::RemoveClips {
                clip_ids: sound,
                ripple: false,
            })
            .expect("removes");
        assert_eq!(
            editor
                .project()
                .active()
                .clip(&clip_id)
                .expect("video")
                .muted,
            Some(true)
        );
        editor
            .apply(Command::UpdateClip {
                clip_id: clip_id.clone(),
                patch: ClipPatch {
                    muted: Some(false),
                    ..Default::default()
                },
            })
            .expect("unmutes");
        assert_eq!(
            editor
                .project()
                .active()
                .clip(&clip_id)
                .expect("video")
                .muted,
            None,
            "unmuted is the absent value"
        );
    }

    #[test]
    fn a_command_carrying_nan_is_refused_whole() {
        let (mut editor, _, clip_id) = fixture();
        let before = editor.project().clone();
        for command in [
            Command::SetClipSpeed {
                clip_id: clip_id.clone(),
                speed: f64::NAN,
            },
            Command::SplitClips {
                clip_ids: vec![clip_id.clone()],
                time: f64::NAN,
            },
            Command::SetClipTransform {
                clip_id: clip_id.clone(),
                scale: None,
                offset_x: Some(f64::INFINITY),
                offset_y: None,
                rotation: None,
                stretch_x: None,
                stretch_y: None,
            },
            Command::Batch {
                commands: vec![Command::TrimClip {
                    clip_id: clip_id.clone(),
                    edge: TrimEdge::End,
                    delta: f64::NAN,
                    ripple: false,
                }],
            },
        ] {
            assert_eq!(
                editor.apply(command).expect_err("refused"),
                crate::CommandError::NotANumber
            );
        }
        assert_eq!(editor.project(), &before, "nothing changed");
    }

    #[test]
    fn nan_in_transition_or_text_patch_is_refused() {
        let (mut editor, _, clip_id) = fixture();
        let before = editor.project().clone();
        // A transition whose duration is NaN must be caught.
        let bad_transition = Command::UpdateClip {
            clip_id: clip_id.clone(),
            patch: ClipPatch {
                transition_in: Some(Some(crate::model::Transition {
                    id: "cross-fade".to_owned(),
                    duration: f64::NAN,
                })),
                ..Default::default()
            },
        };
        assert_eq!(
            editor.apply(bad_transition).expect_err("refused"),
            crate::CommandError::NotANumber
        );
        // A text style whose font_size is infinite must be caught.
        let style = TextStyle {
            font_size: f64::INFINITY,
            ..Default::default()
        };
        let bad_text = Command::UpdateClip {
            clip_id: clip_id.clone(),
            patch: ClipPatch {
                text: Some(Some(style)),
                ..Default::default()
            },
        };
        assert_eq!(
            editor.apply(bad_text).expect_err("refused"),
            crate::CommandError::NotANumber
        );
        assert_eq!(editor.project(), &before, "nothing changed");
    }

    #[test]
    fn a_document_from_a_newer_build_is_refused() {
        let (editor, _, _) = fixture();
        let mut document = editor.to_document(&settings());
        document["version"] = serde_json::json!(crate::DOCUMENT_VERSION + 1);
        assert!(Editor::from_document(&document).is_none());
        assert_eq!(
            crate::document_version(&document),
            crate::DOCUMENT_VERSION + 1
        );
        assert_eq!(crate::document_version(&serde_json::json!({})), 1);
    }

    #[test]
    fn a_loaded_document_holds_the_same_ranges_every_command_does() {
        let (editor, _, _) = fixture();
        let mut document = editor.to_document(&settings());
        let clip = &mut document["timelines"][0]["clips"][0];
        clip["duration"] = serde_json::json!(0.001);
        clip["scale"] = serde_json::json!(50.0);
        clip["offsetX"] = serde_json::json!(-9.0);
        clip["rotation"] = serde_json::json!(540.0);
        clip["speed"] = serde_json::json!(100.0);
        let loaded = Editor::from_document(&document).expect("loads");
        let clip = &loaded.project().active().clips[0];
        assert_eq!(clip.duration, 1.0 / 60.0);
        assert_eq!(clip.scale, 8.0);
        assert_eq!(clip.offset_x, -3.0);
        assert_eq!(clip.rotation, 180.0);
        assert_eq!(clip.speed, 16.0);
    }

    #[test]
    fn a_renamed_timeline_does_not_count_towards_the_next_number() {
        let (mut editor, _, _) = fixture();
        editor.apply(Command::AddTimeline).expect("adds");
        let second = editor.project().timelines[1].id.clone();
        editor
            .apply(Command::RenameTimeline {
                timeline_id: second,
                name: "Timeline 2 v10".to_owned(),
            })
            .expect("renames");
        editor.apply(Command::AddTimeline).expect("adds");
        assert_eq!(editor.project().timelines[2].name, "Timeline 2");
    }

    #[test]
    fn a_gesture_is_one_undo_step() {
        let (mut editor, _, clip_id) = fixture();
        let start = editor.project().active().clips[0].scale;
        for scale in [1.1, 1.2, 1.3] {
            editor
                .apply_within(
                    Some("scale"),
                    Command::SetClipTransform {
                        clip_id: clip_id.clone(),
                        scale: Some(scale),
                        offset_x: None,
                        offset_y: None,
                        rotation: None,
                        stretch_x: None,
                        stretch_y: None,
                    },
                )
                .expect("scales");
        }
        editor.end_gesture();
        editor
            .apply_within(
                Some("scale"),
                Command::SetClipTransform {
                    clip_id: clip_id.clone(),
                    scale: Some(2.0),
                    offset_x: None,
                    offset_y: None,
                    rotation: None,
                    stretch_x: None,
                    stretch_y: None,
                },
            )
            .expect("scales again");
        assert_eq!(editor.project().active().clips[0].scale, 2.0);
        // Two steps: the drag, then the second drag after the gesture ended.
        assert!(editor.undo());
        assert_eq!(editor.project().active().clips[0].scale, 1.3);
        assert!(editor.undo());
        assert_eq!(editor.project().active().clips[0].scale, start);
        assert!(editor.redo());
        assert_eq!(editor.project().active().clips[0].scale, 1.3);
    }

    #[test]
    fn switching_and_reordering_tabs_is_not_an_edit() {
        let (mut editor, _, _) = fixture();
        editor.apply(Command::AddTimeline).expect("adds");
        let (first, second) = (
            editor.project().timelines[0].id.clone(),
            editor.project().timelines[1].id.clone(),
        );
        let steps_before = {
            let mut count = 0;
            while editor.undo() {
                count += 1;
            }
            while editor.redo() {}
            count
        };
        editor
            .apply(Command::SelectTimeline {
                timeline_id: first.clone(),
            })
            .expect("selects");
        editor
            .apply(Command::MoveTimeline {
                timeline_id: second.clone(),
                index: 0,
            })
            .expect("moves");
        assert_eq!(editor.project().active_timeline_id, first);
        assert_eq!(editor.project().timelines[0].id, second);
        let mut count = 0;
        while editor.undo() {
            count += 1;
        }
        assert_eq!(
            count, steps_before,
            "neither the switch nor the move was a step"
        );
    }

    /// A ripple delete moves the clips behind the gap and copies those
    /// alone: the clip in front is still the snapshot's.
    #[test]
    fn a_ripple_copies_only_the_clips_it_moves() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::SplitClips {
                clip_ids: vec![clip_id.clone()],
                time: 4.0,
            })
            .expect("splits");
        editor
            .apply(Command::SplitClips {
                clip_ids: vec![editor.project().active().clips[1].id.clone()],
                time: 7.0,
            })
            .expect("splits again");
        let head = std::sync::Arc::clone(&editor.project().active().clips[0]);
        let middle = editor.project().active().clips[1].id.clone();
        let last = std::sync::Arc::clone(&editor.project().active().clips[2]);
        editor
            .apply(Command::RemoveClips {
                clip_ids: vec![middle],
                ripple: true,
            })
            .expect("ripple deletes");
        let clips = &editor.project().active().clips;
        assert_eq!(clips.len(), 2);
        assert!(
            std::sync::Arc::ptr_eq(&head, &clips[0]),
            "the clip in front of the gap is still shared with the snapshot"
        );
        assert!(
            !std::sync::Arc::ptr_eq(&last, &clips[1]),
            "the moved clip was copied"
        );
        assert_eq!(clips[1].start, 4.0);
    }

    /// Every command's result goes through `Clip::tidy`: a key set past the
    /// field's range comes out clamped, the way the field itself would.
    #[test]
    fn a_command_leaves_a_tidy_clip_behind() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::SetClipKey {
                clip_id: clip_id.clone(),
                property: crate::model::KeyProperty::OffsetX,
                at: 0.5,
                value: 99.0,
                ease: Default::default(),
            })
            .expect("sets a key");
        let clip = &editor.project().active().clips[0];
        let key = clip
            .keys_on(crate::model::KeyProperty::OffsetX)
            .next()
            .expect("the key");
        assert_eq!(
            key.value,
            crate::model::ranges::MAX_OFFSET,
            "clamped like the field"
        );
    }

    #[test]
    fn a_command_copies_only_what_it_writes() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::SplitClips {
                clip_ids: vec![clip_id.clone()],
                time: 4.0,
            })
            .expect("splits");
        editor.apply(Command::AddTimeline).expect("adds");
        editor
            .apply(Command::SelectTimeline {
                timeline_id: "TL1".to_owned(),
            })
            .expect("back to the first");
        let other = std::sync::Arc::clone(&editor.project().timelines[1]);
        let untouched = std::sync::Arc::clone(&editor.project().active().clips[1]);
        editor
            .apply(Command::UpdateClip {
                clip_id: clip_id.clone(),
                patch: ClipPatch {
                    volume: Some(0.5),
                    ..Default::default()
                },
            })
            .expect("edits the head");
        assert!(
            std::sync::Arc::ptr_eq(&other, &editor.project().timelines[1]),
            "the other timeline is still shared with the snapshot"
        );
        assert!(
            std::sync::Arc::ptr_eq(&untouched, &editor.project().active().clips[1]),
            "the clip the edit did not touch is still shared"
        );
        assert_eq!(editor.project().active().clips[0].volume, 0.5);
        assert!(editor.undo());
        assert_eq!(editor.project().active().clips[0].volume, 1.0);
    }

    #[test]
    fn fields_this_build_does_not_know_survive_a_round_trip() {
        let (editor, _, clip_id) = fixture();
        let mut document = editor.to_document(&settings());
        document["futureSetting"] = json!({ "on": true });
        document["timelines"][0]["colourSpace"] = json!("rec709");
        document["timelines"][0]["tracks"][0]["name"] = json!("Dialogue");
        document["timelines"][0]["clips"][0]["mask"] = json!({ "shape": "ellipse" });
        document["media"][0]["proxy"] = json!("/proxies/a.mp4");

        let loaded = Editor::from_document(&document).expect("loads");
        assert_eq!(
            loaded
                .project()
                .active()
                .clip(&clip_id)
                .expect("clip")
                .extra["mask"]["shape"],
            "ellipse"
        );
        let saved = loaded.to_document(&settings());
        assert_eq!(saved["futureSetting"]["on"], true);
        assert_eq!(saved["timelines"][0]["colourSpace"], "rec709");
        assert_eq!(saved["timelines"][0]["tracks"][0]["name"], "Dialogue");
        assert_eq!(
            saved["timelines"][0]["clips"][0]["mask"]["shape"],
            "ellipse"
        );
        assert_eq!(saved["media"][0]["proxy"], "/proxies/a.mp4");
        // The flat mirror carries the clip's unknown field too: it is the
        // same clip serialised twice.
        assert_eq!(saved["clips"][0]["mask"]["shape"], "ellipse");
        // And the writer's own fields are never duplicated from `extra`.
        assert_eq!(
            loaded.project().extra.len(),
            1,
            "{:?}",
            loaded.project().extra
        );
    }

    #[test]
    fn a_timeline_without_its_own_frame_takes_the_documents() {
        let document = json!({
            "name": "Old", "version": 1,
            "video": { "width": 1080, "height": 1920, "rateNum": 60, "rateDen": 1 },
            "media": [],
            "timelines": [
                { "id": "TL1", "name": "Vertical",
                  "tracks": [{ "id": "T1", "visible": true, "muted": false }], "clips": [] },
                { "id": "TL2", "name": "Half", "video": { "width": 0, "height": 540 },
                  "tracks": [{ "id": "T2", "visible": true, "muted": false }], "clips": [] }
            ],
            "activeTimelineId": "TL2"
        });
        let editor = Editor::from_document(&document).expect("loads");
        let timelines = &editor.project().timelines;
        assert_eq!(
            (timelines[0].video.width, timelines[0].video.height),
            (1080, 1920)
        );
        assert_eq!(timelines[0].video.rate_num, 60);
        // A zero width takes the document's; a stated height is kept.
        assert_eq!(
            (timelines[1].video.width, timelines[1].video.height),
            (1080, 540)
        );
        assert_eq!(editor.project().active_timeline_id, "TL2");
    }

    #[test]
    fn an_entry_the_reader_cannot_parse_is_dropped_not_the_document() {
        let document = json!({
            "name": "Mixed", "version": 1,
            "media": [
                { "id": "m1", "path": "/a.mp4", "kind": "hologram" },
                { "id": "m2", "path": "/b.mp4", "kind": "audio", "audioTracks": "nope" },
                { "path": "/no-id.mp4" }
            ],
            "tracks": [{ "id": "T1" }, { "visible": false }],
            "clips": [
                { "id": "c1", "trackId": "T1", "mediaId": "m1", "kind": "video",
                  "cutout": { "mode": "unknown" }, "keys": "garbage",
                  "transitionIn": { "id": "cross-fade" },
                  "videoEffects": [{ "id": "sepia", "keys": { "amount": [ { "at": 0.5, "value": 1.0 }, { "at": 7.0, "value": 2.0 } ] } }, "not an effect"] },
                { "id": "c2", "trackId": "T1", "mediaId": "m2", "kind": "audio", "start": "soon" }
            ]
        });
        let editor = Editor::from_document(&document).expect("loads");
        let project = editor.project();
        assert_eq!(project.media.len(), 2, "the entry without an id is dropped");
        assert_eq!(
            project.media[0].kind,
            MediaKind::Video,
            "an unknown kind is video"
        );
        assert_eq!(
            project.media[0].name, "/a.mp4",
            "a nameless entry is called by its path"
        );
        assert!(
            project.media[1].audio_tracks.is_empty(),
            "a list that is not one is empty"
        );
        let timeline = project.active();
        assert_eq!(
            timeline.tracks.len(),
            1,
            "the lane without an id is dropped"
        );
        let ids: Vec<&str> = timeline.clips.iter().map(|clip| clip.id.as_str()).collect();
        assert_eq!(ids, ["c1"], "a clip whose start is not a number is dropped");
        let clip = timeline.clip("c1").expect("c1");
        assert!(
            clip.cutout.is_none(),
            "a cutout that does not parse is no cutout"
        );
        assert!(clip.keys.is_empty());
        assert_eq!(clip.transition_in.as_ref().expect("kept").duration, 1.0);
        assert_eq!(clip.video_effects.len(), 1);
        assert_eq!(
            clip.video_effects[0].keys["amount"].len(),
            1,
            "the out-of-range key is dropped"
        );
    }

    /// A key marks an instant of the picture. Cutting the clip around it,
    /// trimming the clip under it and joining the pieces back must leave it
    /// on that instant, and a ride cut in two must play, over each piece,
    /// exactly what the whole played there.
    #[test]
    fn keys_stay_on_their_instant_through_split_trim_and_merge() {
        use crate::model::{KeyEase, KeyProperty};
        let (mut editor, _, clip_id) = fixture();
        // Opacity 0 at the head, 1 at the middle (5 s of a 10 s clip), 0.5 at the end.
        for (at, value) in [(0.0, 0.0), (0.5, 1.0), (1.0, 0.5)] {
            editor
                .apply(Command::SetClipKey {
                    clip_id: clip_id.clone(),
                    property: KeyProperty::Opacity,
                    at,
                    value,
                    ease: KeyEase::LINEAR,
                })
                .expect("keys");
        }
        let whole = editor
            .project()
            .active()
            .clip(&clip_id)
            .expect("clip")
            .clone();
        let ride_at = |seconds: f64| whole.value_at(KeyProperty::Opacity, seconds / whole.duration);

        editor
            .apply(Command::SplitClips {
                clip_ids: vec![clip_id.clone()],
                time: 4.0,
            })
            .expect("splits");
        let timeline = editor.project().active();
        let head = timeline.clips[0].clone();
        let tail = timeline.clips[1].clone();
        // The head's ride over its four seconds is the whole's over 0-4 s;
        // the tail's over its six is the whole's over 4-10 s.
        for seconds in [0.0, 1.0, 2.5, 4.0] {
            let got = head.value_at(KeyProperty::Opacity, seconds / head.duration);
            assert!(
                (got - ride_at(seconds)).abs() < 1e-9,
                "head at {seconds}: {got}"
            );
        }
        for seconds in [4.0, 5.0, 7.0, 10.0] {
            let got = tail.value_at(KeyProperty::Opacity, (seconds - 4.0) / tail.duration);
            assert!(
                (got - ride_at(seconds)).abs() < 1e-9,
                "tail at {seconds}: {got}"
            );
        }
        assert_eq!(
            head.keys.len(),
            2,
            "the head: its own key and one on the cut"
        );
        assert_eq!(
            tail.keys.len(),
            3,
            "the tail: one on the cut and its own two"
        );

        // Trimming a second off the tail's head keeps the middle key on
        // the picture's 5 s: now one second into the tail.
        editor
            .apply(Command::TrimClip {
                clip_id: tail.id.clone(),
                edge: TrimEdge::Start,
                delta: 1.0,
                ripple: false,
            })
            .expect("trims");
        let trimmed = editor
            .project()
            .active()
            .clip(&tail.id)
            .expect("tail")
            .clone();
        let middle = trimmed
            .keys_on(KeyProperty::Opacity)
            .find(|key| (key.value - 1.0).abs() < 1e-9)
            .expect("the middle key");
        assert!(
            (middle.at * trimmed.duration - 0.0).abs() < 1e-9,
            "at {}",
            middle.at
        );
        // And back, so the pieces join again.
        editor
            .apply(Command::TrimClip {
                clip_id: tail.id.clone(),
                edge: TrimEdge::Start,
                delta: -1.0,
                ripple: false,
            })
            .expect("trims back");

        editor
            .apply(Command::MergeClips {
                clip_ids: vec![clip_id.clone(), tail.id.clone()],
            })
            .expect("merges");
        let merged = editor
            .project()
            .active()
            .clip(&clip_id)
            .expect("merged")
            .clone();
        assert_eq!(merged.duration, 10.0);
        for seconds in [0.0, 2.0, 4.0, 5.0, 8.0, 10.0] {
            let got = merged.value_at(KeyProperty::Opacity, seconds / merged.duration);
            assert!(
                (got - ride_at(seconds)).abs() < 1e-9,
                "merged at {seconds}: {got}"
            );
        }

        // Lengthening the end spreads nothing: the keys keep their seconds.
        editor
            .apply(Command::TrimClip {
                clip_id: clip_id.clone(),
                edge: TrimEdge::End,
                delta: 10.0,
                ripple: false,
            })
            .expect("extends");
        let longer = editor
            .project()
            .active()
            .clip(&clip_id)
            .expect("clip")
            .clone();
        let middle = longer
            .keys_on(KeyProperty::Opacity)
            .find(|key| (key.value - 1.0).abs() < 1e-9)
            .expect("the middle key");
        assert!((middle.at * longer.duration - 5.0).abs() < 1e-9);
    }

    #[test]
    fn a_split_leaves_the_fade_in_with_the_head_and_the_fade_out_with_the_tail() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::UpdateClip {
                clip_id: clip_id.clone(),
                patch: ClipPatch {
                    fade_in: Some(0.5),
                    fade_out: Some(0.5),
                    ..Default::default()
                },
            })
            .expect("fades");
        editor
            .apply(Command::SplitClips {
                clip_ids: vec![clip_id.clone()],
                time: 4.0,
            })
            .expect("splits");
        let timeline = editor.project().active();
        let (head, tail) = (&timeline.clips[0], &timeline.clips[1]);
        assert_eq!((head.fade_in, head.fade_out), (0.5, 0.0));
        assert_eq!((tail.fade_in, tail.fade_out), (0.0, 0.5));

        let ids: Vec<String> = timeline.clips.iter().map(|clip| clip.id.clone()).collect();
        editor
            .apply(Command::MergeClips { clip_ids: ids })
            .expect("merges");
        let clip = &editor.project().active().clips[0];
        assert_eq!((clip.fade_in, clip.fade_out), (0.5, 0.5));
    }

    #[test]
    fn effect_keys_stay_on_their_instant_through_a_split() {
        use crate::model::{AppliedFilter, KeyEase};
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::UpdateClip {
                clip_id: clip_id.clone(),
                patch: ClipPatch {
                    video_effects: Some(vec![AppliedFilter::new("concat.vignette")]),
                    ..Default::default()
                },
            })
            .expect("applies");
        for (at, value) in [(0.0, 10.0), (0.8, 90.0)] {
            editor
                .apply(Command::SetEffectKey {
                    clip_id: clip_id.clone(),
                    entry: 0,
                    key: "strength".to_owned(),
                    at,
                    value,
                    ease: KeyEase::LINEAR,
                })
                .expect("keys");
        }
        editor
            .apply(Command::SplitClips {
                clip_ids: vec![clip_id.clone()],
                time: 4.0,
            })
            .expect("splits");
        let timeline = editor.project().active();
        let (head, tail) = (&timeline.clips[0], &timeline.clips[1]);
        // The ride is 10 → 90 over 0-8 s: 50 at the cut.
        let at_cut_head = head.video_effects[0].value_at("strength", 1.0, 0.0);
        let at_cut_tail = tail.video_effects[0].value_at("strength", 0.0, 0.0);
        assert!((at_cut_head - 50.0).abs() < 1e-9, "{at_cut_head}");
        assert!((at_cut_tail - 50.0).abs() < 1e-9, "{at_cut_tail}");
        // The tail's second key is still on the picture's 8 s: four seconds in.
        let ninety = tail.video_effects[0]
            .keys_on("strength")
            .iter()
            .find(|key| (key.value - 90.0).abs() < 1e-9)
            .expect("the 90 key");
        assert!((ninety.at * tail.duration - 4.0).abs() < 1e-9);
    }

    #[test]
    fn rearranged_pieces_refuse_to_merge() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::SplitClips {
                clip_ids: vec![clip_id.clone()],
                time: 4.0,
            })
            .expect("splits");
        let tail_id = editor.project().active().clips[1].id.clone();
        let track_id = editor.project().active().tracks[0].id.clone();
        // Swap the two pieces on the timeline: adjacent, but out of order.
        editor
            .apply(Command::MoveClips {
                moves: vec![
                    ClipMove {
                        clip_id: clip_id.clone(),
                        start: 6.0,
                        track_id: track_id.clone(),
                    },
                    ClipMove {
                        clip_id: tail_id.clone(),
                        start: 0.0,
                        track_id,
                    },
                ],
            })
            .expect("moves");
        let refused = editor.apply(Command::MergeClips {
            clip_ids: vec![tail_id, clip_id],
        });
        assert!(
            refused
                .unwrap_err()
                .to_string()
                .contains("no longer in their original order")
        );
    }

    #[test]
    fn a_head_trim_moves_the_in_point_with_speed() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::SetClipSpeed {
                clip_id: clip_id.clone(),
                speed: 2.0,
            })
            .expect("ok");
        // 10s of source at 2x occupies 5s of timeline.
        assert_eq!(editor.project().active().clips[0].duration, 5.0);
        editor
            .apply(Command::TrimClip {
                clip_id,
                edge: TrimEdge::Start,
                delta: 1.0,
                ripple: false,
            })
            .expect("trims");
        let clip = &editor.project().active().clips[0];
        assert_eq!(clip.start, 1.0);
        assert_eq!(clip.duration, 4.0);
        assert_eq!(
            clip.source_start, 2.0,
            "a timeline second covers two source seconds"
        );
    }

    #[test]
    fn speed_clamps_to_the_engine_range() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::SetClipSpeed {
                clip_id: clip_id.clone(),
                speed: 100.0,
            })
            .expect("ok");
        assert_eq!(editor.project().active().clips[0].speed, 16.0);
        editor
            .apply(Command::SetClipSpeed {
                clip_id,
                speed: 0.0,
            })
            .expect("ok");
        assert_eq!(editor.project().active().clips[0].speed, 0.0625);
    }

    #[test]
    fn detach_and_reattach_round_trip_the_sound() {
        let (mut editor, _, clip_id) = fixture();
        let sound_id = editor
            .apply(Command::DetachAudio {
                clip_id: clip_id.clone(),
            })
            .expect("detaches")
            .created_id
            .expect("sound clip");

        let timeline = editor.project().active();
        assert_eq!(timeline.clips.len(), 2);
        let sound = timeline.clip(&sound_id).expect("exists");
        assert_eq!(sound.kind, ClipKind::Audio);
        assert_eq!(sound.detached_from.as_deref(), Some(clip_id.as_str()));
        assert_eq!(timeline.clip(&clip_id).expect("exists").muted, Some(true));

        editor
            .apply(Command::ReattachAudio { clip_id: sound_id })
            .expect("reattaches");
        let timeline = editor.project().active();
        assert_eq!(timeline.clips.len(), 1);
        assert_eq!(timeline.clip(&clip_id).expect("exists").muted, None);
    }

    /// A file with two audio tracks detaches as two sound clips, one per
    /// track, each on its own lane and named for its track; reattaching
    /// takes both back. A clip's chosen track survives the document.
    #[test]
    fn detaching_a_two_track_recording_gives_one_sound_clip_per_track() {
        let mut editor = Editor::new();
        let media_id = editor
            .apply(two_track_media("/rec.mkv"))
            .expect("adds")
            .created_id
            .expect("id");
        assert_eq!(editor.project().media[0].audio_tracks.len(), 2);
        let track_id = editor.project().active().tracks[0].id.clone();
        let clip_id = editor
            .apply(Command::AddClip {
                media_id: media_id.clone(),
                track_id,
                start: 0.0,
                ripple: false,
            })
            .expect("adds")
            .created_id
            .expect("id");

        editor
            .apply(Command::DetachAudio {
                clip_id: clip_id.clone(),
            })
            .expect("detaches");
        let timeline = editor.project().active();
        let sounds: Vec<&crate::model::Clip> = timeline
            .clips
            .iter()
            .map(|clip| clip.as_ref())
            .filter(|clip| clip.kind == ClipKind::Audio)
            .collect();
        assert_eq!(sounds.len(), 2);
        assert_eq!(sounds[0].audio_stream, Some(1));
        assert_eq!(sounds[1].audio_stream, Some(2));
        assert!(sounds[0].name.ends_with("Track 1"), "{}", sounds[0].name);
        assert!(sounds[1].name.ends_with("Mic/Aux"), "{}", sounds[1].name);
        assert_ne!(
            sounds[0].track_id, sounds[1].track_id,
            "each on its own lane"
        );
        assert!(
            sounds
                .iter()
                .all(|sound| sound.detached_from.as_deref() == Some(clip_id.as_str()))
        );

        // The document keeps the choice and the list.
        let saved = crate::doc::to_document(&settings(), editor.project());
        let loaded = crate::doc::from_document(&saved).expect("loads");
        assert_eq!(
            loaded.media[0].audio_tracks,
            editor.project().media[0].audio_tracks
        );
        let back: Vec<Option<u32>> = loaded
            .active()
            .clips
            .iter()
            .filter(|clip| clip.kind == ClipKind::Audio)
            .map(|clip| clip.audio_stream)
            .collect();
        assert_eq!(back, vec![Some(1), Some(2)]);
        assert_eq!(loaded.media[0].audio_track_position(Some(2)), 1);
        assert_eq!(loaded.media[0].audio_track_position(None), 0);
        assert_eq!(loaded.media[0].audio_track_position(Some(9)), 0);

        editor
            .apply(Command::ReattachAudio {
                clip_id: sounds[1].id.clone(),
            })
            .expect("reattaches");
        let timeline = editor.project().active();
        assert_eq!(timeline.clips.len(), 1);
        assert_eq!(timeline.clip(&clip_id).expect("exists").muted, None);

        // A video clip can be told which track to play, and told to forget.
        editor
            .apply(Command::UpdateClip {
                clip_id: clip_id.clone(),
                patch: ClipPatch {
                    audio_stream: Some(Some(2)),
                    ..Default::default()
                },
            })
            .expect("picks");
        assert_eq!(
            editor
                .project()
                .active()
                .clip(&clip_id)
                .unwrap()
                .audio_stream,
            Some(2)
        );
        editor
            .apply(Command::UpdateClip {
                clip_id: clip_id.clone(),
                patch: ClipPatch {
                    audio_stream: Some(None),
                    ..Default::default()
                },
            })
            .expect("forgets");
        assert_eq!(
            editor
                .project()
                .active()
                .clip(&clip_id)
                .unwrap()
                .audio_stream,
            None
        );
    }

    #[test]
    fn timelines_add_switch_and_delete_with_a_floor_of_one() {
        let mut editor = Editor::new();
        let second = editor
            .apply(Command::AddTimeline)
            .expect("adds")
            .created_id
            .expect("id");
        assert_eq!(editor.project().active_timeline_id, second);
        assert_eq!(editor.project().timelines[1].name, "Timeline 2");
        assert!(
            editor.project().timelines[1]
                .tracks
                .iter()
                .all(|track| track.id != "T1"),
            "fresh lanes must not reuse the first timeline's ids"
        );

        editor
            .apply(Command::SelectTimeline {
                timeline_id: "TL1".to_owned(),
            })
            .expect("ok");
        assert_eq!(editor.project().active_timeline_id, "TL1");

        editor
            .apply(Command::RemoveTimeline {
                timeline_id: "TL1".to_owned(),
            })
            .expect("ok");
        assert_eq!(editor.project().active_timeline_id, second);
        let last = editor.project().timelines[0].id.clone();
        let refused = editor.apply(Command::RemoveTimeline { timeline_id: last });
        assert!(refused.is_err(), "the last timeline cannot be deleted");
    }

    #[test]
    fn a_timeline_moves_to_a_new_slot_without_stealing_the_selection() {
        let mut editor = Editor::new();
        let second = editor
            .apply(Command::AddTimeline)
            .expect("adds")
            .created_id
            .expect("id");
        let third = editor
            .apply(Command::AddTimeline)
            .expect("adds")
            .created_id
            .expect("id");
        editor
            .apply(Command::SelectTimeline {
                timeline_id: "TL1".to_owned(),
            })
            .expect("ok");

        editor
            .apply(Command::MoveTimeline {
                timeline_id: third.clone(),
                index: 0,
            })
            .expect("moves");
        let order: Vec<_> = editor
            .project()
            .timelines
            .iter()
            .map(|timeline| timeline.id.clone())
            .collect();
        assert_eq!(order, vec![third.clone(), "TL1".to_owned(), second.clone()]);
        assert_eq!(
            editor.project().active_timeline_id,
            "TL1",
            "moving must not select"
        );

        // An index past the end clamps to the last slot; a same-slot move and
        // an unknown id apply nothing, so neither records history.
        editor
            .apply(Command::MoveTimeline {
                timeline_id: third.clone(),
                index: 99,
            })
            .expect("ok");
        let order: Vec<_> = editor
            .project()
            .timelines
            .iter()
            .map(|timeline| timeline.id.clone())
            .collect();
        assert_eq!(order, vec!["TL1".to_owned(), second, third]);
        let before_can_undo = editor.can_undo();
        let outcome = editor
            .apply(Command::MoveTimeline {
                timeline_id: "nope".to_owned(),
                index: 0,
            })
            .expect("no-op");
        assert!(!outcome.applied);
        assert_eq!(editor.can_undo(), before_can_undo);
    }

    #[test]
    fn undo_and_redo_walk_the_history() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::SplitClips {
                clip_ids: vec![clip_id],
                time: 5.0,
            })
            .expect("splits");
        assert_eq!(editor.project().active().clips.len(), 2);

        assert!(editor.undo());
        assert_eq!(editor.project().active().clips.len(), 1);
        assert!(editor.redo());
        assert_eq!(editor.project().active().clips.len(), 2);
        assert!(editor.undo() && editor.undo() && editor.undo());
        assert!(
            editor.project().media.is_empty(),
            "all the way back to empty"
        );
    }

    #[test]
    fn a_failed_command_records_no_history() {
        let (mut editor, _, _) = fixture();
        let before_can_undo = editor.can_undo();
        let _ = editor.apply(Command::MergeClips {
            clip_ids: vec!["nope".to_owned()],
        });
        assert_eq!(editor.can_undo(), before_can_undo);
    }

    #[test]
    fn the_document_round_trips() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::UpdateClip {
                clip_id: clip_id.clone(),
                patch: ClipPatch {
                    volume: Some(0.5),
                    transition_in: Some(Some(crate::model::Transition {
                        id: "cross-fade".to_owned(),
                        duration: 1.5,
                    })),
                    ..ClipPatch::default()
                },
            })
            .expect("updates");
        editor.apply(Command::AddTimeline).expect("adds");

        let document = editor.to_document(&settings());
        let restored = Editor::from_document(&document).expect("loads");
        assert_eq!(restored.project(), editor.project());
    }

    #[test]
    fn a_version_one_document_loads() {
        // The shape of a version-1 document as it sits on disk, optional
        // fields omitted - documents like this exist and must load forever.
        let document = json!({
            "concat": "0.1.0", "version": 1, "name": "v1",
            "video": { "width": 1920, "height": 1080, "rateNum": 30, "rateDen": 1 },
            "media": [{ "id": "m1", "path": "/a.mp4", "name": "a.mp4", "duration": 10.0,
                        "kind": "video", "width": 1920, "height": 1080, "frameRate": 30.0,
                        "frameRateFraction": "30/1", "videoCodec": "h264",
                        "audioCodec": null, "hasAudio": false }],
            "tracks": [{ "id": "T1", "name": "Track 1", "visible": true, "muted": false }],
            "clips": [{ "id": "c1", "trackId": "T1", "mediaId": "m1", "name": "a.mp4",
                        "kind": "video", "start": 0.0, "duration": 10.0, "sourceStart": 0.0,
                        "volume": 1.0, "fadeIn": 0.0, "fadeOut": 0.0, "scale": 1.0,
                        "offsetX": 0.0, "offsetY": 0.0, "rotation": 0.0, "opacity": 1.0,
                        "speed": 1.0, "preservePitch": true, "filters": [],
                        "videoEffects": [{ "id": "sepia", "params": {}, "enabled": true }] }],
            "fonts": [],
            "timelines": [{ "id": "TL1", "name": "Timeline 1",
                "tracks": [{ "id": "T1", "name": "Track 1", "visible": true, "muted": false }],
                "clips": [{ "id": "c1", "trackId": "T1", "mediaId": "m1", "name": "a.mp4",
                            "kind": "video", "start": 0.0, "duration": 10.0, "sourceStart": 0.0,
                            "volume": 1.0, "fadeIn": 0.0, "fadeOut": 0.0, "scale": 1.0,
                            "offsetX": 0.0, "offsetY": 0.0, "rotation": 0.0, "opacity": 1.0,
                            "speed": 1.0, "preservePitch": true, "filters": [],
                            "videoEffects": [{ "id": "sepia", "params": {}, "enabled": true }] }] }],
            "activeTimelineId": "TL1"
        });
        let editor = Editor::from_document(&document).expect("loads");
        let project = editor.project();
        assert_eq!(project.timelines.len(), 1);
        assert_eq!(project.active().clips[0].video_effects[0].id, "sepia");
    }

    /// Keys were shipped with four named eases before the curve editor
    /// replaced them with control points. Both spellings have to load, or a
    /// project saved a week ago opens with its animation gone straight.
    #[test]
    fn a_key_reads_its_ease_named_or_numbered() {
        let document = json!({
            "name": "Keyed", "version": 1,
            "media": [{ "id": "m1", "path": "/a.mp4", "name": "a.mp4",
                        "kind": "video", "duration": 10.0 }],
            "tracks": [{ "id": "T1", "name": "Track 1", "visible": true, "muted": false }],
            "clips": [{ "id": "c1", "trackId": "T1", "mediaId": "m1", "name": "a.mp4",
                        "kind": "video", "start": 0.0, "duration": 10.0, "sourceStart": 0.0,
                        "volume": 1.0, "fadeIn": 0.0, "fadeOut": 0.0, "scale": 1.0,
                        "offsetX": 0.0, "offsetY": 0.0, "rotation": 0.0, "opacity": 1.0,
                        "speed": 1.0, "preservePitch": true, "filters": [],
                        "keys": [
                            { "property": "opacity", "at": 0.0, "value": 0.0, "ease": "inOut" },
                            { "property": "opacity", "at": 0.5, "value": 1.0,
                              "ease": [0.1, 0.2, 0.3, 0.4] },
                            { "property": "opacity", "at": 1.0, "value": 0.5 },
                            { "property": "opacity", "at": 9.0, "value": 1.0 },
                            { "property": "nonesuch", "at": 0.2, "value": 1.0 }
                        ] }]
        });
        let editor = Editor::from_document(&document).expect("loads");
        let keys = &editor.project().active().clips[0].keys;

        // The out-of-range key and the one naming an unknown property are
        // dropped; the other three survive in order.
        assert_eq!(keys.len(), 3);
        assert!(keys.windows(2).all(|pair| pair[0].at <= pair[1].at));

        assert_eq!(keys[0].ease, crate::model::KeyEase::IN_OUT);
        assert_eq!(keys[1].ease, crate::model::KeyEase([0.1, 0.2, 0.3, 0.4]));
        // Absent is a straight line, not a refusal to load.
        assert_eq!(keys[2].ease, crate::model::KeyEase::LINEAR);
    }

    #[test]
    fn a_legacy_flat_document_loads_as_one_timeline() {
        let document = json!({
            "name": "Old", "version": 1,
            "media": [],
            "tracks": [{ "id": "T1", "name": "Track 1", "visible": true, "muted": false }],
            "clips": []
        });
        let editor = Editor::from_document(&document).expect("loads");
        assert_eq!(editor.project().timelines.len(), 1);
        assert_eq!(editor.project().timelines[0].name, "Timeline 1");
    }

    #[test]
    fn restored_ids_can_never_be_reissued() {
        let (editor, _, _) = fixture();
        let document = editor.to_document(&settings());
        let mut restored = Editor::from_document(&document).expect("loads");
        let media_id = restored
            .apply(media("/b.mp4", 5.0, false))
            .expect("adds")
            .created_id
            .expect("id");
        assert!(
            restored
                .project()
                .media
                .iter()
                .filter(|item| item.id == media_id)
                .count()
                == 1
                && editor
                    .project()
                    .media
                    .iter()
                    .all(|item| item.id != media_id),
            "the fresh id collides with nothing"
        );
    }

    #[test]
    fn commands_arrive_in_camel_case() {
        // The format the window speaks. A snake_case field here means a
        // caller silently sends values serde never sees.
        let command: Command = serde_json::from_value(json!({
            "op": "addClip", "mediaId": "m1", "trackId": "T1", "start": 2.0
        }))
        .expect("camelCase fields deserialize");
        assert!(matches!(command, Command::AddClip { .. }));

        let transform: Command = serde_json::from_value(json!({
            "op": "setClipTransform", "clipId": "c1", "offsetX": 0.5
        }))
        .expect("optional camelCase fields deserialize");
        assert!(matches!(transform, Command::SetClipTransform { .. }));

        // The template commands ride the same wire.
        let mark: Command = serde_json::from_value(json!({
            "op": "setMediaPlaceholder", "mediaId": "m1", "placeholder": true
        }))
        .expect("parses");
        assert!(matches!(
            mark,
            Command::SetMediaPlaceholder {
                placeholder: true,
                ..
            }
        ));

        // A colour range is one of two words, or null to read the tag.
        let full: Command = serde_json::from_value(json!({
            "op": "setMediaColorRange", "mediaId": "m1", "range": "full"
        }))
        .expect("parses");
        assert!(matches!(
            full,
            Command::SetMediaColorRange {
                range: Some(crate::model::ColorRange::Full),
                ..
            }
        ));
        let tagged: Command = serde_json::from_value(json!({
            "op": "setMediaColorRange", "mediaId": "m1", "range": null
        }))
        .expect("parses");
        assert!(matches!(
            tagged,
            Command::SetMediaColorRange { range: None, .. }
        ));
        assert!(
            serde_json::from_value::<Command>(json!({
                "op": "setMediaColorRange", "mediaId": "m1", "range": "wide"
            }))
            .is_err(),
            "a range is limited or full"
        );

        let batch: Command = serde_json::from_value(json!({
            "op": "batch",
            "commands": [{ "op": "fillSlot", "mediaId": "m1", "item": {
                "path": "/b.mp4", "name": "b.mp4", "duration": 3.0, "kind": "video",
                "width": null, "height": null, "frameRate": null, "frameRateFraction": null,
                "videoCodec": null, "audioCodec": null, "hasAudio": false
            }}]
        }))
        .expect("parses");
        assert!(matches!(
            &batch,
            Command::Batch { commands } if matches!(commands[0], Command::FillSlot { .. })
        ));

        // ClipPatch double-options: absent leaves alone, null clears.
        let patch: crate::commands::ClipPatch =
            serde_json::from_value(json!({ "transitionIn": null })).expect("parses");
        assert_eq!(patch.transition_in, Some(None));
        let untouched: crate::commands::ClipPatch =
            serde_json::from_value(json!({})).expect("parses");
        assert_eq!(untouched.transition_in, None);
    }

    /// A media item's colour range is set, saved, cleared, and undone
    /// like any edit; an unknown item is a no-op that records nothing.
    /// https://github.com/jub0t/Concat/issues/103
    #[test]
    fn a_medias_colour_range_is_an_undoable_edit_that_round_trips() {
        use crate::model::ColorRange;
        let (mut editor, media_id, _clip_id) = fixture();
        assert_eq!(editor.project().media[0].color_range, None);
        let before = editor.project().clone();

        let view = editor
            .apply(Command::SetMediaColorRange {
                media_id: media_id.clone(),
                range: Some(ColorRange::Full),
            })
            .expect("sets");
        assert!(view.applied);
        assert_eq!(
            editor.project().media[0].color_range,
            Some(ColorRange::Full)
        );

        // Saved as the word, read back as the range; absent stays absent,
        // so a project that never said stays byte-identical.
        let saved = editor.to_document(&settings());
        assert_eq!(saved["media"][0]["colorRange"], "full");
        let reopened = Editor::from_document(&saved).expect("loads");
        assert_eq!(
            reopened.project().media[0].color_range,
            Some(ColorRange::Full)
        );
        let untold = crate::doc::to_document(&settings(), &before);
        assert!(untold["media"][0].get("colorRange").is_none());
        // A word this build does not know reads as the tag, not as a
        // media item lost.
        let mut odd = saved.clone();
        odd["media"][0]["colorRange"] = json!("wide");
        let tolerant = Editor::from_document(&odd).expect("loads");
        assert_eq!(tolerant.project().media.len(), 1);
        assert_eq!(tolerant.project().media[0].color_range, None);

        // Cleared is back to the tag; set to what it is already, nothing.
        editor
            .apply(Command::SetMediaColorRange {
                media_id: media_id.clone(),
                range: None,
            })
            .expect("clears");
        assert_eq!(editor.project().media[0].color_range, None);
        let same = editor
            .apply(Command::SetMediaColorRange {
                media_id: media_id.clone(),
                range: None,
            })
            .expect("no-op");
        assert!(!same.applied, "a value already held changes nothing");
        let gone = editor
            .apply(Command::SetMediaColorRange {
                media_id: "m999".to_owned(),
                range: Some(ColorRange::Limited),
            })
            .expect("an unknown item is a no-op");
        assert!(!gone.applied);
        assert_eq!(gone.created_id, None);

        editor.undo();
        assert_eq!(
            editor.project().media[0].color_range,
            Some(ColorRange::Full),
            "undo of the clear brings the range back"
        );
    }

    #[test]
    fn a_slot_fills_in_place_keeping_the_template_timing() {
        let (mut editor, media_id, clip_id) = fixture();
        // Shape the clip the way a template slot looks: trimmed off the front,
        // so it has a real in-point to forget.
        editor
            .apply(Command::TrimClip {
                clip_id,
                edge: TrimEdge::Start,
                delta: 2.0,
                ripple: false,
            })
            .expect("trims");
        editor
            .apply(Command::SetMediaPlaceholder {
                media_id: media_id.clone(),
                placeholder: true,
            })
            .expect("marks");
        assert!(editor.project().media[0].placeholder);

        editor
            .apply(Command::FillSlot {
                media_id: media_id.clone(),
                item: crate::commands::NewMedia {
                    path: "/photo.jpg".to_owned(),
                    name: "photo.jpg".to_owned(),
                    duration: None,
                    kind: MediaKind::Image,
                    width: Some(4000),
                    height: Some(3000),
                    frame_rate: None,
                    frame_rate_fraction: None,
                    video_codec: None,
                    audio_codec: None,
                    has_audio: false,
                    audio_tracks: Vec::new(),
                    origin: None,
                },
            })
            .expect("fills");

        let project = editor.project();
        let media = &project.media[0];
        assert!(!media.placeholder, "a filled slot is ordinary media again");
        assert_eq!(media.id, media_id, "the id survives, so clips keep working");
        assert_eq!(media.path, "/photo.jpg");

        let clip = &project.active().clips[0];
        assert_eq!(clip.start, 2.0, "slot position is the template's");
        assert_eq!(clip.duration, 8.0, "slot length is the template's");
        assert_eq!(
            clip.source_start, 0.0,
            "the in-point referred to the old footage"
        );
        assert_eq!(clip.kind, ClipKind::Image);
        assert_eq!(clip.name, "photo.jpg");
    }

    #[test]
    fn filling_ordinary_media_is_refused() {
        let (mut editor, media_id, _) = fixture();
        let refused = editor.apply(Command::FillSlot {
            media_id,
            item: crate::commands::NewMedia {
                path: "/b.mp4".to_owned(),
                name: "b.mp4".to_owned(),
                duration: Some(3.0),
                kind: MediaKind::Video,
                width: None,
                height: None,
                frame_rate: None,
                frame_rate_fraction: None,
                video_codec: None,
                audio_codec: None,
                has_audio: false,
                audio_tracks: Vec::new(),
                origin: None,
            },
        });
        assert!(
            refused
                .unwrap_err()
                .to_string()
                .contains("not a template slot")
        );
    }

    #[test]
    fn a_batch_is_one_undo_step() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::Batch {
                commands: vec![
                    Command::SplitClips {
                        clip_ids: vec![clip_id.clone()],
                        time: 4.0,
                    },
                    Command::SetClipSpeed {
                        clip_id,
                        speed: 2.0,
                    },
                ],
            })
            .expect("applies");
        assert_eq!(editor.project().active().clips.len(), 2);
        assert_eq!(editor.project().active().clips[0].speed, 2.0);

        assert!(editor.undo());
        let clips = &editor.project().active().clips;
        assert_eq!(clips.len(), 1, "one undo reverses the whole batch");
        assert_eq!(clips[0].speed, 1.0);
    }

    #[test]
    fn a_failed_batch_changes_nothing() {
        let (mut editor, _, clip_id) = fixture();
        let before = editor.project().clone();
        let refused = editor.apply(Command::Batch {
            commands: vec![
                Command::SplitClips {
                    clip_ids: vec![clip_id],
                    time: 4.0,
                },
                Command::MergeClips {
                    clip_ids: vec!["nope".to_owned()],
                },
            ],
        });
        assert!(refused.is_err());
        assert_eq!(
            editor.project(),
            &before,
            "the successful half was rolled back"
        );

        assert!(editor.undo());
        assert!(
            editor.project().active().clips.is_empty(),
            "the first undo steps over the fixture's add-clip, so the batch recorded nothing"
        );
    }

    #[test]
    fn the_placeholder_flag_round_trips_and_defaults_off() {
        let (mut editor, media_id, _) = fixture();
        editor
            .apply(Command::SetMediaPlaceholder {
                media_id,
                placeholder: true,
            })
            .expect("marks");

        let document = editor.to_document(&settings());
        let restored = Editor::from_document(&document).expect("loads");
        assert!(
            restored.project().media[0].placeholder,
            "the flag survives the document"
        );

        // A document from a build that predates templates has no such field.
        let legacy = json!({
            "name": "Old", "version": 1,
            "media": [{ "id": "m1", "path": "/a.mp4", "name": "a.mp4", "kind": "video",
                        "hasAudio": false }],
            "tracks": [{ "id": "T1", "name": "Track 1", "visible": true, "muted": false }],
            "clips": []
        });
        let editor = Editor::from_document(&legacy).expect("loads");
        assert!(!editor.project().media[0].placeholder);
    }

    #[test]
    fn a_medias_origin_round_trips_and_an_unknown_one_reads_as_an_import() {
        let mut editor = Editor::new();
        let Command::AddMedia { mut item } = media("/voice.wav", 3.0, true) else {
            unreachable!()
        };
        item.kind = MediaKind::Audio;
        item.origin = Some(MediaOrigin::Speech);
        editor.apply(Command::AddMedia { item }).expect("adds");
        editor.apply(media("/a.mp4", 10.0, true)).expect("adds");

        let document = editor.to_document(&settings());
        let media = document["media"].as_array().expect("a list");
        assert_eq!(media[0]["origin"], json!("speech"));
        assert!(
            media[1].get("origin").is_none(),
            "an import says nothing, so a document without voices stays byte-identical"
        );
        let restored = Editor::from_document(&document).expect("loads");
        assert_eq!(
            restored.project().media[0].origin,
            Some(MediaOrigin::Speech)
        );
        assert_eq!(restored.project().media[1].origin, None);

        // A document from a build that predates origins, and one from a
        // build with origins this one has not heard of: both load, and
        // both files are imports here.
        let legacy = json!({
            "name": "Old", "version": 1,
            "media": [
                { "id": "m1", "path": "/a.mp4", "name": "a.mp4", "kind": "video",
                  "hasAudio": false },
                { "id": "m2", "path": "/b.wav", "name": "b.wav", "kind": "audio",
                  "hasAudio": true, "origin": "telepathy" },
                { "id": "m3", "path": "/c.wav", "name": "c.wav", "kind": "audio",
                  "hasAudio": true, "origin": 7 }
            ],
            "tracks": [{ "id": "T1", "name": "Track 1", "visible": true, "muted": false }],
            "clips": []
        });
        let editor = Editor::from_document(&legacy).expect("loads");
        assert!(
            editor
                .project()
                .media
                .iter()
                .all(|item| item.origin.is_none())
        );
    }

    #[test]
    fn a_garbage_document_is_none_not_a_panic() {
        assert!(Editor::from_document(&json!("nonsense")).is_none());
        assert!(Editor::from_document(&json!({ "tracks": [] })).is_none());
    }

    #[test]
    fn text_clips_survive_without_media_and_carry_their_words() {
        let mut editor = Editor::new();
        let clip_id = editor
            .apply(Command::AddTextClip {
                above: false,
                track_id: None,
                start: 2.0,
                style: Some(TextStyle {
                    content: "Lower third\nsecond line".to_owned(),
                    ..TextStyle::default()
                }),
                duration: Some(2.5),
                offset_y: Some(0.38),
            })
            .expect("adds")
            .created_id
            .expect("id");
        {
            let clip = editor.project().active().clip(&clip_id).expect("exists");
            assert_eq!(clip.name, "Lower third");
            assert_eq!(clip.duration, 2.5, "a stated duration lands directly");
            assert_eq!(clip.offset_y, 0.38, "and so does the placement");
        }

        // The pre-templates wire shape still parses: both fields default.
        let bare: Command = serde_json::from_value(json!({
            "op": "addTextClip", "trackId": null, "start": 1.0, "style": null
        }))
        .expect("old wire shape parses");
        assert!(matches!(
            bare,
            Command::AddTextClip {
                duration: None,
                offset_y: None,
                ..
            }
        ));

        let document = editor.to_document(&settings());
        let restored = Editor::from_document(&document).expect("loads");
        assert_eq!(restored.project(), editor.project());
    }

    #[test]
    fn a_caption_lands_above_the_highest_occupied_lane() {
        let mut editor = Editor::new();
        let media_id = editor
            .apply(media("/a.mp4", 10.0, true))
            .expect("adds")
            .created_id
            .expect("id");
        let video_track = editor.project().active().tracks[2].id.clone();
        editor
            .apply(Command::AddClip {
                media_id,
                track_id: video_track.clone(),
                start: 0.0,
                ripple: false,
            })
            .expect("adds");

        let caption_id = editor
            .apply(Command::AddTextClip {
                track_id: None,
                above: true,
                start: 0.0,
                style: Some(TextStyle {
                    content: "hi".to_owned(),
                    ..TextStyle::default()
                }),
                duration: Some(5.0),
                offset_y: None,
            })
            .expect("adds")
            .created_id
            .expect("id");

        let caption = editor.project().active().clip(&caption_id).expect("exists");
        let tracks = &editor.project().active().tracks;
        let caption_row = tracks
            .iter()
            .position(|t| t.id == caption.track_id)
            .expect("row");
        let video_row = tracks
            .iter()
            .position(|t| t.id == video_track)
            .expect("row");
        assert!(
            caption_row > video_row,
            "a caption must land above the video, not under it"
        );
    }

    #[test]
    fn a_caption_on_an_empty_timeline_lands_on_the_bottom_lane() {
        let mut editor = Editor::new();
        let caption_id = editor
            .apply(Command::AddTextClip {
                track_id: None,
                above: true,
                start: 0.0,
                style: Some(TextStyle {
                    content: "hi".to_owned(),
                    ..TextStyle::default()
                }),
                duration: Some(5.0),
                offset_y: None,
            })
            .expect("adds")
            .created_id
            .expect("id");

        let caption = editor.project().active().clip(&caption_id).expect("exists");
        let tracks = &editor.project().active().tracks;
        let caption_row = tracks
            .iter()
            .position(|t| t.id == caption.track_id)
            .expect("row");
        assert_eq!(
            caption_row, 0,
            "with nothing occupied, the caption takes the bottom lane"
        );
    }

    #[test]
    fn a_plain_title_still_lands_on_the_first_free_lane_from_the_bottom() {
        let mut editor = Editor::new();
        let media_id = editor
            .apply(media("/a.mp4", 10.0, true))
            .expect("adds")
            .created_id
            .expect("id");
        let video_track = editor.project().active().tracks[2].id.clone();
        editor
            .apply(Command::AddClip {
                media_id,
                track_id: video_track,
                start: 0.0,
                ripple: false,
            })
            .expect("adds");

        let title_id = editor
            .apply(Command::AddTextClip {
                track_id: None,
                above: false,
                start: 0.0,
                style: Some(TextStyle {
                    content: "title".to_owned(),
                    ..TextStyle::default()
                }),
                duration: Some(5.0),
                offset_y: None,
            })
            .expect("adds")
            .created_id
            .expect("id");

        let title = editor.project().active().clip(&title_id).expect("exists");
        let tracks = &editor.project().active().tracks;
        let title_row = tracks
            .iter()
            .position(|t| t.id == title.track_id)
            .expect("row");
        assert_eq!(
            title_row, 0,
            "a plain title keeps the old bottom-first behaviour"
        );
    }

    #[test]
    fn a_caption_mints_a_new_lane_when_every_lane_above_is_taken() {
        let mut editor = Editor::new();
        let media_id = editor
            .apply(media("/a.mp4", 10.0, true))
            .expect("adds")
            .created_id
            .expect("id");
        let track_ids: Vec<String> = editor
            .project()
            .active()
            .tracks
            .iter()
            .map(|t| t.id.clone())
            .collect();
        for track_id in &track_ids {
            editor
                .apply(Command::AddClip {
                    media_id: media_id.clone(),
                    track_id: track_id.clone(),
                    start: 0.0,
                    ripple: false,
                })
                .expect("adds");
        }

        let caption_id = editor
            .apply(Command::AddTextClip {
                track_id: None,
                above: true,
                start: 0.0,
                style: Some(TextStyle {
                    content: "hi".to_owned(),
                    ..TextStyle::default()
                }),
                duration: Some(5.0),
                offset_y: None,
            })
            .expect("adds")
            .created_id
            .expect("id");

        let caption = editor.project().active().clip(&caption_id).expect("exists");
        let tracks = &editor.project().active().tracks;
        assert_eq!(
            tracks.len(),
            track_ids.len() + 1,
            "one lane was minted at the top for the caption"
        );
        let caption_row = tracks
            .iter()
            .position(|t| t.id == caption.track_id)
            .expect("row");
        assert_eq!(
            caption_row,
            track_ids.len(),
            "the caption sits on the newly minted top lane"
        );
    }

    #[test]
    fn a_tolerated_no_op_records_no_undo_entry() {
        // The applied flag replaced a deep state compare; if a missing-id
        // command ever reported true, undo would gain phantom steps.
        let (mut editor, _, _) = fixture();
        let outcome = editor
            .apply(Command::TrimClip {
                clip_id: "nope".to_owned(),
                edge: TrimEdge::End,
                delta: 1.0,
                ripple: false,
            })
            .expect("tolerated");
        assert!(!outcome.applied);

        assert!(editor.undo(), "history still holds the fixture's edits");
        assert_eq!(
            editor.project().active().clips.len(),
            0,
            "the first undo steps over the add-clip, not a phantom no-op"
        );
    }

    #[test]
    fn setting_a_value_already_in_place_records_no_undo_entry() {
        let (mut editor, _, clip_id) = fixture();
        // Volume is already 1.0; the old deep-compare saw no change here and
        // pushed nothing - the applied flag must agree exactly.
        let outcome = editor
            .apply(Command::UpdateClip {
                clip_id,
                patch: ClipPatch {
                    volume: Some(1.0),
                    ..ClipPatch::default()
                },
            })
            .expect("tolerated");
        assert!(!outcome.applied);
        assert!(editor.undo());
        assert!(
            editor.project().active().clips.is_empty(),
            "straight back past the add-clip"
        );
    }

    #[test]
    fn a_batch_reports_applied_only_when_a_member_changed_something() {
        let (mut editor, _, clip_id) = fixture();
        let track_id = editor.project().active().tracks[0].id.clone();
        // Visible is already true; only the speed change is real.
        let outcome = editor
            .apply(Command::Batch {
                commands: vec![
                    Command::SetTrackFlag {
                        track_id: track_id.clone(),
                        flag: TrackFlag::Visible,
                        value: true,
                    },
                    Command::SetClipSpeed {
                        clip_id,
                        speed: 2.0,
                    },
                ],
            })
            .expect("applies");
        assert!(outcome.applied);
        assert!(editor.undo());
        assert_eq!(
            editor.project().active().clips[0].speed,
            1.0,
            "one undo undoes the batch"
        );

        // A batch of nothing-but-no-ops is itself a no-op: no undo entry.
        let outcome = editor
            .apply(Command::Batch {
                commands: vec![
                    Command::SetTrackFlag {
                        track_id,
                        flag: TrackFlag::Visible,
                        value: true,
                    },
                    Command::TrimClip {
                        clip_id: "nope".to_owned(),
                        edge: TrimEdge::End,
                        delta: 1.0,
                        ripple: false,
                    },
                ],
            })
            .expect("tolerated");
        assert!(!outcome.applied);
        assert!(editor.undo());
        assert!(
            editor.project().active().clips.is_empty(),
            "the next undo steps over the add-clip, not a phantom batch"
        );
    }

    #[test]
    fn undo_depth_evicts_the_oldest_snapshot_first() {
        // The cap used to remove(0) on a Vec; the VecDeque must keep the
        // same outward behaviour - the newest 200 steps stay undoable.
        let (mut editor, _, clip_id) = fixture();
        for step in 0..205 {
            editor
                .apply(Command::SetClipTransform {
                    clip_id: clip_id.clone(),
                    scale: None,
                    offset_x: Some(f64::from(step) / 1000.0 + 0.001),
                    offset_y: None,
                    rotation: None,
                    stretch_x: None,
                    stretch_y: None,
                })
                .expect("applies");
        }
        let mut undos = 0;
        while editor.undo() {
            undos += 1;
        }
        assert_eq!(undos, 200);
    }

    #[test]
    fn moves_to_a_vanished_track_keep_the_time_change_and_drop_the_track_change() {
        // Pinned deliberately: a drag that races a track deletion still
        // lands its horizontal half.
        let (mut editor, _, clip_id) = fixture();
        let outcome = editor
            .apply(Command::MoveClips {
                moves: vec![ClipMove {
                    clip_id: clip_id.clone(),
                    start: 3.0,
                    track_id: "gone".to_owned(),
                }],
            })
            .expect("tolerated");
        assert!(outcome.applied, "the start change is real");
        let clip = editor.project().active().clip(&clip_id).expect("exists");
        assert_eq!(clip.start, 3.0);
        assert_eq!(clip.track_id, "T1", "the vanished destination is ignored");
    }

    #[test]
    fn a_multi_move_skips_unknown_clips_and_floors_start_at_zero() {
        let (mut editor, _, clip_id) = fixture();
        let target = editor.project().active().tracks[1].id.clone();
        editor
            .apply(Command::MoveClips {
                moves: vec![
                    // The unknown clip is skipped; the rest still lands.
                    ClipMove {
                        clip_id: "nope".to_owned(),
                        start: 9.0,
                        track_id: target.clone(),
                    },
                    ClipMove {
                        clip_id: clip_id.clone(),
                        start: -2.0,
                        track_id: target.clone(),
                    },
                ],
            })
            .expect("tolerated");
        let clip = editor.project().active().clip(&clip_id).expect("exists");
        assert_eq!(
            clip.start, 0.0,
            "negative starts clamp to the timeline head"
        );
        assert_eq!(clip.track_id, target);
    }

    #[test]
    fn a_tail_trim_stretches_only_the_duration_and_stops_at_the_minimum() {
        let (mut editor, _, clip_id) = fixture();
        editor
            .apply(Command::TrimClip {
                clip_id: clip_id.clone(),
                edge: TrimEdge::End,
                delta: -4.0,
                ripple: false,
            })
            .expect("trims");
        let clip = editor.project().active().clip(&clip_id).expect("exists");
        assert_eq!(clip.duration, 6.0);
        assert_eq!(clip.start, 0.0, "the head does not move");
        assert_eq!(clip.source_start, 0.0, "nor the in-point");

        // Dragging far past the head floors at a sixtieth of a second
        // rather than inverting the clip.
        editor
            .apply(Command::TrimClip {
                clip_id: clip_id.clone(),
                edge: TrimEdge::End,
                delta: -100.0,
                ripple: false,
            })
            .expect("trims");
        assert_eq!(
            editor
                .project()
                .active()
                .clip(&clip_id)
                .expect("exists")
                .duration,
            1.0 / 60.0
        );
    }

    #[test]
    fn add_at_first_free_takes_the_lowest_empty_lane_or_the_bottom_one() {
        let (mut editor, media_id, _) = fixture();
        // Track one is occupied for [0, 10), so the same span lands on two.
        let second = editor
            .apply(Command::AddClipAtFirstFree {
                media_id: media_id.clone(),
                start: 0.0,
            })
            .expect("adds")
            .created_id
            .expect("id");
        assert_eq!(
            editor
                .project()
                .active()
                .clip(&second)
                .expect("exists")
                .track_id,
            editor.project().active().tracks[1].id
        );

        // Fill the remaining lanes, then ask again: rather than refusing,
        // the clip overlaps on the first track.
        editor
            .apply(Command::AddClipAtFirstFree {
                media_id: media_id.clone(),
                start: 0.0,
            })
            .expect("adds");
        editor
            .apply(Command::AddClipAtFirstFree {
                media_id: media_id.clone(),
                start: 0.0,
            })
            .expect("adds");
        let overflow = editor
            .apply(Command::AddClipAtFirstFree {
                media_id: media_id.clone(),
                start: 0.0,
            })
            .expect("adds")
            .created_id
            .expect("id");
        assert_eq!(
            editor
                .project()
                .active()
                .clip(&overflow)
                .expect("exists")
                .track_id,
            editor.project().active().tracks[0].id,
            "a full timeline falls back to the first track, overlap and all"
        );

        // A clear span later in time finds track one free again.
        let clear = editor
            .apply(Command::AddClipAtFirstFree {
                media_id,
                start: 20.0,
            })
            .expect("adds")
            .created_id
            .expect("id");
        assert_eq!(
            editor
                .project()
                .active()
                .clip(&clear)
                .expect("exists")
                .track_id,
            editor.project().active().tracks[0].id
        );
    }

    #[test]
    fn removing_a_track_takes_its_clips_and_stops_at_the_floor_of_one() {
        let (mut editor, _, _) = fixture();
        let track_id = editor.project().active().tracks[0].id.clone();
        editor
            .apply(Command::RemoveTrack {
                track_id: track_id.clone(),
            })
            .expect("removes");
        let timeline = editor.project().active();
        assert_eq!(timeline.tracks.len(), 3);
        assert!(
            timeline.clips.is_empty(),
            "the clip on the removed track went with it"
        );

        // An unknown id is the tolerated no-op, not an error.
        let outcome = editor
            .apply(Command::RemoveTrack { track_id })
            .expect("tolerated");
        assert!(!outcome.applied);

        while editor.project().active().tracks.len() > 1 {
            let next = editor.project().active().tracks[0].id.clone();
            editor
                .apply(Command::RemoveTrack { track_id: next })
                .expect("removes");
        }
        let last = editor.project().active().tracks[0].id.clone();
        let refused = editor.apply(Command::RemoveTrack { track_id: last });
        assert_eq!(
            refused.unwrap_err().to_string(),
            "A timeline needs at least one track."
        );
    }

    #[test]
    fn removing_media_sweeps_its_clips_from_every_timeline() {
        let (mut editor, media_id, _) = fixture();
        editor.apply(Command::AddTimeline).expect("adds");
        let track_id = editor.project().active().tracks[0].id.clone();
        editor
            .apply(Command::AddClip {
                media_id: media_id.clone(),
                track_id,
                start: 0.0,
                ripple: false,
            })
            .expect("adds");

        editor
            .apply(Command::RemoveMedia {
                media_id: media_id.clone(),
            })
            .expect("removes");
        assert!(editor.project().media.is_empty());
        assert!(
            editor
                .project()
                .timelines
                .iter()
                .all(|timeline| timeline.clips.is_empty()),
            "the inactive timeline's clip is swept too, not left dangling"
        );

        // Removing what is already gone is a tolerated no-op.
        let outcome = editor
            .apply(Command::RemoveMedia { media_id })
            .expect("tolerated");
        assert!(!outcome.applied);
    }

    #[test]
    fn renames_trim_whitespace_and_refuse_to_blank_a_name() {
        let (mut editor, _, _) = fixture();
        editor
            .apply(Command::RenameTimeline {
                timeline_id: "TL1".to_owned(),
                name: "\tCut A\n".to_owned(),
            })
            .expect("renames");
        assert_eq!(editor.project().timelines[0].name, "Cut A");
        let outcome = editor
            .apply(Command::RenameTimeline {
                timeline_id: "TL1".to_owned(),
                name: String::new(),
            })
            .expect("tolerated");
        assert!(!outcome.applied);
        assert_eq!(editor.project().timelines[0].name, "Cut A");
    }

    #[test]
    fn track_flags_flip_independently_and_report_no_ops_honestly() {
        let (mut editor, _, _) = fixture();
        let track_id = editor.project().active().tracks[0].id.clone();
        editor
            .apply(Command::SetTrackFlag {
                track_id: track_id.clone(),
                flag: TrackFlag::Visible,
                value: false,
            })
            .expect("sets");
        editor
            .apply(Command::SetTrackFlag {
                track_id: track_id.clone(),
                flag: TrackFlag::Muted,
                value: true,
            })
            .expect("sets");
        let track = &editor.project().active().tracks[0];
        assert!(!track.visible);
        assert!(track.muted);

        // Setting the value already in place is a no-op, exactly as the old
        // deep-compare judged it.
        let outcome = editor
            .apply(Command::SetTrackFlag {
                track_id,
                flag: TrackFlag::Muted,
                value: true,
            })
            .expect("tolerated");
        assert!(!outcome.applied);
    }

    #[test]
    fn rotation_wraps_into_the_half_open_degree_range() {
        // (-180, 180]: a knob dragged through full turns must not carry the
        // turns with it, and the boundary itself belongs to +180.
        let (mut editor, _, clip_id) = fixture();
        let cases = [
            (361.0, 1.0),
            (-1.0, -1.0),
            (720.0, 0.0),
            (180.0, 180.0),
            (-180.0, 180.0),
            (540.0, 180.0),
        ];
        for (sent, landed) in cases {
            editor
                .apply(Command::SetClipTransform {
                    clip_id: clip_id.clone(),
                    scale: None,
                    offset_x: None,
                    offset_y: None,
                    rotation: Some(sent),
                    stretch_x: None,
                    stretch_y: None,
                })
                .expect("sets");
            assert_eq!(
                editor
                    .project()
                    .active()
                    .clip(&clip_id)
                    .expect("exists")
                    .rotation,
                landed,
                "{sent} degrees should land at {landed}"
            );
        }
    }

    #[test]
    fn removing_a_timeline_moves_the_active_tab_to_a_neighbour() {
        let mut editor = Editor::new();
        let second = editor
            .apply(Command::AddTimeline)
            .expect("adds")
            .created_id
            .expect("id");
        let third = editor
            .apply(Command::AddTimeline)
            .expect("adds")
            .created_id
            .expect("id");
        assert_eq!(editor.project().active_timeline_id, third);

        // An unknown id is a tolerated no-op while the floor allows it.
        let outcome = editor
            .apply(Command::RemoveTimeline {
                timeline_id: "ghost".to_owned(),
            })
            .expect("tolerated");
        assert!(!outcome.applied);

        // Removing the active last tab falls back to the previous one.
        editor
            .apply(Command::RemoveTimeline { timeline_id: third })
            .expect("removes");
        assert_eq!(editor.project().active_timeline_id, second);

        // Removing an active middle tab prefers the neighbour to its right.
        let third = editor
            .apply(Command::AddTimeline)
            .expect("adds")
            .created_id
            .expect("id");
        editor
            .apply(Command::SelectTimeline {
                timeline_id: second.clone(),
            })
            .expect("selects");
        editor
            .apply(Command::RemoveTimeline {
                timeline_id: second,
            })
            .expect("removes");
        assert_eq!(editor.project().active_timeline_id, third);

        // Removing an inactive tab leaves the selection alone.
        editor
            .apply(Command::RemoveTimeline {
                timeline_id: "TL1".to_owned(),
            })
            .expect("removes");
        assert_eq!(editor.project().active_timeline_id, third);
    }

    #[test]
    fn fonts_deduplicate_by_path_and_removal_leaves_titles_alone() {
        let mut editor = Editor::new();
        editor
            .apply(Command::AddFont {
                family: "Inter".to_owned(),
                path: "/fonts/inter.ttf".to_owned(),
            })
            .expect("adds");
        // Same path again: a re-import must not duplicate the entry.
        let outcome = editor
            .apply(Command::AddFont {
                family: "Inter Again".to_owned(),
                path: "/fonts/inter.ttf".to_owned(),
            })
            .expect("tolerated");
        assert!(!outcome.applied);
        assert_eq!(editor.project().fonts.len(), 1);

        let clip_id = editor
            .apply(Command::AddTextClip {
                above: false,
                track_id: None,
                start: 0.0,
                style: Some(TextStyle {
                    font_family: "Inter".to_owned(),
                    ..TextStyle::default()
                }),
                duration: None,
                offset_y: None,
            })
            .expect("adds")
            .created_id
            .expect("id");

        editor
            .apply(Command::RemoveFont {
                family: "Inter".to_owned(),
            })
            .expect("removes");
        assert!(editor.project().fonts.is_empty());
        let clip = editor.project().active().clip(&clip_id).expect("exists");
        assert_eq!(
            clip.text.as_ref().expect("text").font_family,
            "Inter",
            "the title keeps the family name so the face can come back"
        );

        // Removing what is already gone is a tolerated no-op.
        let outcome = editor
            .apply(Command::RemoveFont {
                family: "Inter".to_owned(),
            })
            .expect("tolerated");
        assert!(!outcome.applied);
    }

    #[test]
    fn every_command_round_trips_through_serde() {
        // One of each variant. Grow this list with the enum, or a new
        // command ships without proof its wire shape survives a round trip.
        let item = match media("/a.mp4", 10.0, true) {
            Command::AddMedia { item } => item,
            _ => unreachable!("the helper builds AddMedia"),
        };
        let commands = vec![
            Command::AddMedia { item: item.clone() },
            Command::RemoveMedia {
                media_id: "m1".to_owned(),
            },
            Command::SetMediaPlaceholder {
                media_id: "m1".to_owned(),
                placeholder: true,
            },
            Command::FillSlot {
                media_id: "m1".to_owned(),
                item,
            },
            Command::Batch {
                commands: vec![Command::AddTrack],
            },
            Command::AddClip {
                media_id: "m1".to_owned(),
                track_id: "T1".to_owned(),
                start: 1.0,
                ripple: false,
            },
            Command::AddClipAtFirstFree {
                media_id: "m1".to_owned(),
                start: 2.0,
            },
            Command::AddTextClip {
                above: false,
                track_id: Some("T1".to_owned()),
                start: 0.0,
                style: Some(TextStyle::default()),
                duration: Some(2.0),
                offset_y: Some(0.4),
            },
            Command::MoveClips {
                moves: vec![ClipMove {
                    clip_id: "c1".to_owned(),
                    start: 0.0,
                    track_id: "T2".to_owned(),
                }],
            },
            Command::TrimClip {
                clip_id: "c1".to_owned(),
                edge: TrimEdge::End,
                delta: -0.5,
                ripple: false,
            },
            Command::SplitClips {
                clip_ids: vec!["c1".to_owned()],
                time: 3.0,
            },
            Command::MergeClips {
                clip_ids: vec!["c1".to_owned(), "c2".to_owned()],
            },
            Command::RemoveClips {
                clip_ids: vec!["c1".to_owned()],
                ripple: false,
            },
            Command::UpdateClip {
                clip_id: "c1".to_owned(),
                patch: ClipPatch {
                    volume: Some(0.5),
                    transition_in: Some(None),
                    ..ClipPatch::default()
                },
            },
            Command::SetClipSpeed {
                clip_id: "c1".to_owned(),
                speed: 2.0,
            },
            Command::SetClipTransform {
                clip_id: "c1".to_owned(),
                scale: Some(1.5),
                offset_x: None,
                offset_y: Some(-0.25),
                rotation: Some(90.0),
                stretch_x: None,
                stretch_y: None,
            },
            Command::DetachAudio {
                clip_id: "c1".to_owned(),
            },
            Command::ReattachAudio {
                clip_id: "c1".to_owned(),
            },
            Command::AddTrack,
            Command::RemoveTrack {
                track_id: "T1".to_owned(),
            },
            Command::SetTrackFlag {
                track_id: "T1".to_owned(),
                flag: TrackFlag::Muted,
                value: true,
            },
            Command::AddTimeline,
            Command::RemoveTimeline {
                timeline_id: "TL1".to_owned(),
            },
            Command::RenameTimeline {
                timeline_id: "TL1".to_owned(),
                name: "Cut A".to_owned(),
            },
            Command::SelectTimeline {
                timeline_id: "TL1".to_owned(),
            },
            Command::AddFont {
                family: "Inter".to_owned(),
                path: "/fonts/inter.ttf".to_owned(),
            },
            Command::RemoveFont {
                family: "Inter".to_owned(),
            },
        ];
        for command in commands {
            let wire = serde_json::to_value(&command).expect("serialises");
            let back: Command = serde_json::from_value(wire).expect("parses");
            assert_eq!(back, command);
        }
    }

    #[test]
    fn clips_for_vanished_tracks_or_media_are_dropped_on_load() {
        // A hand-edited or truncated file must degrade to something
        // openable: a clip with nowhere to live (or nothing to show) is
        // dropped, while a text clip - which needs no media - survives.
        let document = json!({
            "name": "Damaged", "version": 1,
            "media": [{ "id": "m1", "path": "/a.mp4", "name": "a.mp4", "kind": "video",
                        "hasAudio": false }],
            "tracks": [{ "id": "T1", "name": "Track 1", "visible": true, "muted": false }],
            "clips": [
                { "id": "c1", "trackId": "ghost", "mediaId": "m1", "kind": "video",
                  "start": 0.0, "duration": 1.0 },
                { "id": "c2", "trackId": "T1", "mediaId": "ghost", "kind": "video",
                  "start": 0.0, "duration": 1.0 },
                { "id": "c3", "trackId": "T1", "mediaId": "m1", "kind": "video",
                  "start": 0.0, "duration": 1.0 },
                { "id": "c4", "trackId": "T1", "kind": "text", "start": 0.0, "duration": 1.0 }
            ]
        });
        let editor = Editor::from_document(&document).expect("loads");
        let ids: Vec<&str> = editor
            .project()
            .active()
            .clips
            .iter()
            .map(|clip| clip.id.as_str())
            .collect();
        assert_eq!(ids, ["c3", "c4"]);
    }

    #[test]
    fn hand_edited_values_are_clamped_on_load() {
        // The reader trusts nothing: values that would render invisibly or
        // fall outside the engine's ranges are pulled back in, not obeyed.
        let document = json!({
            "name": "Edited", "version": 1,
            "media": [{ "id": "m1", "path": "/a.mp4", "name": "a.mp4", "kind": "video",
                        "hasAudio": false }],
            "tracks": [{ "id": "T1", "name": "Track 1", "visible": true, "muted": false }],
            "clips": [
                { "id": "c1", "trackId": "T1", "mediaId": "m1", "kind": "video",
                  "start": -4.0, "duration": -3.0, "sourceStart": -1.0,
                  "volume": -2.0, "opacity": 2.0, "speed": 100.0, "scale": 0.0,
                  "transitionIn": { "id": "cross-fade", "duration": 0.0 } },
                { "id": "c2", "trackId": "T1", "kind": "text", "start": 0.0, "duration": 1.0,
                  "text": { "content": "Hi", "fontSize": 0.0, "fontWeight": 9999.0,
                            "lineHeight": 0.1, "opacity": 5.0 } }
            ]
        });
        let editor = Editor::from_document(&document).expect("loads");
        let clip = &editor.project().active().clips[0];
        assert_eq!(clip.start, 0.0);
        assert_eq!(clip.duration, 1.0 / 60.0, "the floor every command holds");
        assert_eq!(clip.source_start, 0.0);
        assert_eq!(clip.volume, 0.0);
        assert_eq!(clip.opacity, 1.0);
        assert_eq!(clip.speed, 16.0);
        assert_eq!(clip.scale, 0.05);
        assert_eq!(clip.transition_in.as_ref().expect("kept").duration, 0.1);

        let text = editor.project().active().clips[1]
            .text
            .as_ref()
            .expect("text");
        assert_eq!(
            text.font_size, 0.01,
            "a zero size would render an invisible title"
        );
        assert_eq!(text.font_weight, 900.0);
        assert_eq!(
            text.line_height, 0.5,
            "lines cannot collapse onto each other"
        );
        assert_eq!(text.opacity, 1.0);
    }

    #[test]
    fn effect_keys_ride_a_knob_and_survive_the_document() {
        use crate::model::{AppliedFilter, KeyEase};
        let (mut editor, _, clip_id) = fixture();
        let mut link = AppliedFilter::new("concat.adjust");
        link.params.insert("exposure".to_owned(), 1.0);
        editor
            .apply(Command::UpdateClip {
                clip_id: clip_id.clone(),
                patch: ClipPatch {
                    video_effects: Some(vec![link]),
                    ..ClipPatch::default()
                },
            })
            .expect("applies");
        for (at, value) in [(0.0, -1.0), (1.0, 1.0)] {
            editor
                .apply(Command::SetEffectKey {
                    clip_id: clip_id.clone(),
                    entry: 0,
                    key: "exposure".to_owned(),
                    at,
                    value,
                    ease: KeyEase::LINEAR,
                })
                .expect("keys");
        }
        let link = &editor
            .project()
            .active()
            .clip(&clip_id)
            .expect("clip")
            .video_effects[0];
        assert!(link.is_keyed("exposure"));
        assert!(!link.is_keyed("contrast"));
        // Halfway along a straight ride between -1 and 1 is 0; the constant
        // is what the knob falls back to once the keys come off.
        assert!((link.value_at("exposure", 0.5, 0.0)).abs() < 1e-9);
        assert_eq!(link.params_at(0.25).get("exposure").copied(), Some(-0.5));
        assert_eq!(link.keys_around("exposure", 0.5), (Some(0.0), Some(1.0)));
        assert!(link.key_at("exposure", 1.0).is_some());

        let document = editor.to_document(&settings());
        let restored = Editor::from_document(&document).expect("loads");
        let link = &restored
            .project()
            .active()
            .clip(&clip_id)
            .expect("clip")
            .video_effects[0];
        assert_eq!(link.keys_on("exposure").len(), 2);

        let mut editor = restored;
        editor
            .apply(Command::ClearEffectKey {
                clip_id: clip_id.clone(),
                entry: 0,
                key: "exposure".to_owned(),
                at: 1.0,
            })
            .expect("clears one");
        editor
            .apply(Command::ClearEffectKeys {
                clip_id: clip_id.clone(),
                entry: 0,
                key: "exposure".to_owned(),
            })
            .expect("clears the rest");
        let link = &editor
            .project()
            .active()
            .clip(&clip_id)
            .expect("clip")
            .video_effects[0];
        assert!(!link.is_keyed("exposure"));
        assert_eq!(link.value_at("exposure", 0.5, 0.0), 1.0);
    }

    #[test]
    fn missing_media_detects_nonexistent_paths() {
        let mut project = Project::new();

        // Dodaj media item s nepostojećom pathom
        let id = "m1".to_owned();
        project.media.push(MediaItem {
            id: id.clone(),
            path: "/does/not/exist.mp4".to_owned(),
            name: "Missing Clip".to_owned(),
            duration: Some(10.0),
            kind: MediaKind::Video,
            width: Some(1920),
            height: Some(1080),
            frame_rate: None,
            frame_rate_fraction: None,
            video_codec: None,
            audio_codec: None,
            has_audio: false,
            audio_tracks: vec![],
            placeholder: false,
            color_range: None,
            origin: None,
            extra: Default::default(),
        });

        let missing = project.missing_media();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].id, "m1");
        assert_eq!(missing[0].name, "Missing Clip");
        assert_eq!(missing[0].path, "/does/not/exist.mp4");
    }

    #[test]
    fn missing_media_ignores_existing_paths() {
        let mut project = Project::new();

        // Koristi Cargo.toml koji sigurno postoji
        let existing = std::env::current_dir()
            .unwrap()
            .join("Cargo.toml")
            .to_string_lossy()
            .to_string();

        project.media.push(MediaItem {
            id: "m2".to_owned(),
            path: existing,
            name: "Existing Clip".to_owned(),
            duration: Some(5.0),
            kind: MediaKind::Video,
            width: Some(1920),
            height: Some(1080),
            frame_rate: None,
            frame_rate_fraction: None,
            video_codec: None,
            audio_codec: None,
            has_audio: false,
            audio_tracks: vec![],
            placeholder: false,
            color_range: None,
            origin: None,
            extra: Default::default(),
        });

        let missing = project.missing_media();
        assert_eq!(missing.len(), 0);
    }
}

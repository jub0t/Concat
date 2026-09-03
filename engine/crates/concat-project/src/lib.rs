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

pub mod animation;
pub mod commands;
pub mod doc;
pub mod editor;
pub mod model;
pub mod speed;

pub use commands::{Command, CommandError, Outcome, why_not_merge};
pub use doc::{DocumentSettings, from_document, to_document};
pub use editor::Editor;
pub use model::Project;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::commands::{ClipMove, ClipPatch, Command, TrackFlag, TrimEdge};
    use crate::doc::DocumentSettings;
    use crate::editor::Editor;
    use crate::model::{ClipKind, MediaKind, TextStyle};

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
            },
        }
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
            })
            .expect("adds")
            .created_id
            .expect("id");
        (editor, media_id, clip_id)
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
    fn a_garbage_document_is_none_not_a_panic() {
        assert!(Editor::from_document(&json!("nonsense")).is_none());
        assert!(Editor::from_document(&json!({ "tracks": [] })).is_none());
    }

    #[test]
    fn text_clips_survive_without_media_and_carry_their_words() {
        let mut editor = Editor::new();
        let clip_id = editor
            .apply(Command::AddTextClip {
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
    fn a_tolerated_no_op_records_no_undo_entry() {
        // The applied flag replaced a deep state compare; if a missing-id
        // command ever reported true, undo would gain phantom steps.
        let (mut editor, _, _) = fixture();
        let outcome = editor
            .apply(Command::TrimClip {
                clip_id: "nope".to_owned(),
                edge: TrimEdge::End,
                delta: 1.0,
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
        let track_id = editor.project().active().tracks[0].id.clone();
        editor
            .apply(Command::RenameTrack {
                track_id: track_id.clone(),
                name: "  Cutaways  ".to_owned(),
            })
            .expect("renames");
        assert_eq!(editor.project().active().tracks[0].name, "Cutaways");

        // Whitespace-only would leave the lane unlabelled, so it is ignored.
        let outcome = editor
            .apply(Command::RenameTrack {
                track_id,
                name: "   ".to_owned(),
            })
            .expect("tolerated");
        assert!(!outcome.applied);
        assert_eq!(editor.project().active().tracks[0].name, "Cutaways");

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
            },
            Command::AddClipAtFirstFree {
                media_id: "m1".to_owned(),
                start: 2.0,
            },
            Command::AddTextClip {
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
            Command::RenameTrack {
                track_id: "T1".to_owned(),
                name: "Cutaways".to_owned(),
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
        assert_eq!(clip.duration, 0.01);
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
}

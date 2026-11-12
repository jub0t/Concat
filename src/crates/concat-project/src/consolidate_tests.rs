// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Merging selected video clips into one rendered clip, as the editor sees
//! it: which selections may be merged and why the others may not, and what
//! the edit that swaps the clips for the rendered file does to the
//! timeline, the bin, undo, batches and the saved project.

use serde_json::json;

use crate::commands::{ClipPatch, NewMedia, TrackFlag};
use crate::doc::DocumentSettings;
use crate::model::{AppliedFilter, Clip, ClipKind, MediaKind, MediaOrigin, Transition};
use crate::{Command, CommandError, Editor, consolidation_clips};

// What a person is told, one sentence per reason.
const TWO_OR_MORE: &str = "Select two or more distinct video clips to merge.";
const GONE: &str = "A selected clip no longer exists.";
const SAME_LANE: &str = "Select video clips on the same track to merge.";
const OVERLAP: &str = "Overlapping clips cannot be merged.";
const DETACHED: &str = "Reattach detached audio before merging its video clips.";
const WHOLE_SPAN: &str = "Select every clip inside the span to merge.";
const FRAME_GRID: &str = "Align clip boundaries to the timeline frame grid before merging.";
const COVERED: &str = "Clips above other visible content cannot be merged into an opaque video.";
const TRANSITION: &str = "Include both sides of a transition in the merge.";
const SHORT_FILE: &str = "The merged video does not cover the selected clips.";

const CLOSE: f64 = 1e-9;

fn file(path: &str, seconds: Option<f64>, kind: MediaKind) -> NewMedia {
    let picture = kind != MediaKind::Audio;
    NewMedia {
        path: path.to_owned(),
        name: path.rsplit('/').next().unwrap_or(path).to_owned(),
        duration: seconds,
        kind,
        width: picture.then_some(1920),
        height: picture.then_some(1080),
        frame_rate: (kind == MediaKind::Video).then_some(30.0),
        frame_rate_fraction: (kind == MediaKind::Video).then(|| "30/1".to_owned()),
        video_codec: picture.then(|| "h264".to_owned()),
        audio_codec: (kind != MediaKind::Image).then(|| "aac".to_owned()),
        has_audio: kind != MediaKind::Image,
        audio_tracks: Vec::new(),
        origin: None,
        color_space: Default::default(),
    }
}

/// The rendered file the window would hand over: a video named as the bin
/// shows merges, made by the editor.
fn rendered(seconds: f64) -> NewMedia {
    let mut item = file(
        "/project/media/merged-1/merged.mp4",
        Some(seconds),
        MediaKind::Video,
    );
    item.name = "Merged clips".to_owned();
    item.origin = Some(MediaOrigin::Processed);
    item
}

/// An editor at 30 fps with a ten-second recording with sound in the bin.
struct Desk {
    editor: Editor,
    media: String,
}

impl Desk {
    fn new() -> Self {
        let mut editor = Editor::new();
        let media = editor
            .apply(Command::AddMedia {
                item: file("/footage/talk.mp4", Some(10.0), MediaKind::Video),
            })
            .expect("the recording joins the bin")
            .created_id
            .expect("a media id");
        Desk { editor, media }
    }

    fn import(&mut self, path: &str, seconds: Option<f64>, kind: MediaKind) -> String {
        self.editor
            .apply(Command::AddMedia {
                item: file(path, seconds, kind),
            })
            .expect("the file joins the bin")
            .created_id
            .expect("a media id")
    }

    /// The id of lane `index`, counted from the bottom.
    fn lane(&self, index: usize) -> String {
        self.editor.project().active().tracks[index].id.clone()
    }

    /// A whole clip of `media` at `start` on lane `lane`.
    fn place_of(&mut self, media: &str, lane: usize, start: f64) -> String {
        let track_id = self.lane(lane);
        self.editor
            .apply(Command::AddClip {
                media_id: media.to_owned(),
                track_id,
                start,
                ripple: false,
            })
            .expect("the clip is placed")
            .created_id
            .expect("a clip id")
    }

    /// A whole clip of the recording (ten seconds) at `start` on `lane`.
    fn place(&mut self, lane: usize, start: f64) -> String {
        let media = self.media.clone();
        self.place_of(&media, lane, start)
    }

    fn change(&mut self, id: &str, edit: impl FnOnce(&mut Clip)) {
        edit(
            self.editor
                .project_mut()
                .active_mut()
                .clip_mut(id)
                .expect("the clip is on the timeline"),
        );
    }

    /// Puts a clip exactly at `start` for `duration` seconds.
    fn span(&mut self, id: &str, start: f64, duration: f64) {
        self.change(id, |clip| {
            clip.start = start;
            clip.duration = duration;
        });
    }

    fn check(&self, ids: &[&String]) -> Result<Vec<Clip>, CommandError> {
        let ids: Vec<String> = ids.iter().map(|id| (*id).clone()).collect();
        consolidation_clips(self.editor.project().active(), &ids)
    }

    fn refusal(&self, ids: &[&String]) -> String {
        self.check(ids)
            .expect_err("the selection cannot be merged")
            .to_string()
    }

    fn accepts(&self, ids: &[&String]) {
        if let Err(error) = self.check(ids) {
            panic!("the selection should be mergeable, but: {error}");
        }
    }

    fn merge(&mut self, ids: &[&String], item: NewMedia) -> Result<Option<String>, CommandError> {
        self.editor
            .apply(Command::ConsolidateClips {
                clip_ids: ids.iter().map(|id| (*id).clone()).collect(),
                item,
            })
            .map(|outcome| outcome.created_id)
    }

    fn clip(&self, id: &str) -> Clip {
        self.editor
            .project()
            .active()
            .clip(id)
            .unwrap_or_else(|| panic!("clip {id} is on the timeline"))
            .clone()
    }

    fn has(&self, id: &str) -> bool {
        self.editor.project().active().clip(id).is_some()
    }
}

fn document_settings() -> DocumentSettings {
    DocumentSettings {
        name: "Merging".to_owned(),
        width: 1920,
        height: 1080,
        rate_num: 30,
        rate_den: 1,
        canvas: [0, 0, 0, 255],
    }
}

// ── which selections may be merged ──

#[test]
fn a_merge_needs_two_different_clips() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 10.0);
    assert_eq!(desk.refusal(&[]), TWO_OR_MORE);
    assert_eq!(desk.refusal(&[&a]), TWO_OR_MORE);
    assert_eq!(desk.refusal(&[&a, &a]), TWO_OR_MORE);
    assert_eq!(
        desk.refusal(&[&a, &b, &a]),
        TWO_OR_MORE,
        "a clip named twice is refused even beside a second clip"
    );
    desk.accepts(&[&a, &b]);
}

#[test]
fn every_selected_clip_must_still_be_on_the_timeline() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let gone = "c-that-was-deleted".to_owned();
    assert_eq!(desk.refusal(&[&a, &gone]), GONE);
}

#[test]
fn only_video_clips_on_one_lane_can_be_merged() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 10.0);
    let upper = desk.lane(1);

    desk.change(&b, |clip| clip.track_id = upper.clone());
    assert_eq!(desk.refusal(&[&a, &b]), SAME_LANE, "two lanes");
    let lower = desk.lane(0);
    desk.change(&b, |clip| clip.track_id = lower.clone());
    desk.accepts(&[&a, &b]);

    for kind in [ClipKind::Image, ClipKind::Audio, ClipKind::Text] {
        desk.change(&b, |clip| clip.kind = kind);
        assert_eq!(
            desk.refusal(&[&a, &b]),
            SAME_LANE,
            "{kind:?} is not a video clip"
        );
    }
}

#[test]
fn clips_on_a_lane_the_timeline_no_longer_has_are_refused() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 10.0);
    for id in [&a, &b] {
        desk.change(id, |clip| clip.track_id = "T-removed".to_owned());
    }
    let error = desk.check(&[&a, &b]).expect_err("the lane is gone");
    assert!(matches!(error, CommandError::TrackGone), "got {error:?}");
}

#[test]
fn overlapping_clips_are_refused_but_touching_ones_are_not() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 10.0);

    desk.span(&b, 9.5, 10.0);
    assert_eq!(desk.refusal(&[&a, &b]), OVERLAP);

    // Less than a microsecond of overlap is a cut, not an overlap.
    desk.span(&b, 10.0 - 5e-7, 10.0);
    desk.accepts(&[&a, &b]);

    desk.span(&b, 10.0, 10.0);
    desk.accepts(&[&a, &b]);
}

#[test]
fn sound_detached_from_a_selected_clip_must_be_reattached_first() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 10.0);
    desk.editor
        .apply(Command::DetachAudio { clip_id: b.clone() })
        .expect("the sound comes off its picture");
    assert_eq!(desk.refusal(&[&a, &b]), DETACHED);
}

#[test]
fn every_clip_inside_the_span_on_the_lane_must_be_selected() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let middle = desk.place(0, 10.0);
    let c = desk.place(0, 20.0);
    let after = desk.place(0, 30.0);

    assert_eq!(desk.refusal(&[&a, &c]), WHOLE_SPAN);
    desk.accepts(&[&a, &middle, &c]);

    // Neighbours touching either edge of the span are outside it.
    desk.accepts(&[&middle, &c]);

    // Once the middle clip is gone, the hole is a gap the merge keeps.
    desk.editor
        .apply(Command::RemoveClips {
            clip_ids: vec![middle.clone()],
            ripple: false,
        })
        .expect("the middle clip is deleted");
    desk.accepts(&[&a, &c]);

    // A neighbour reaching a little way into the span is inside it.
    desk.span(&after, 29.9, 10.0);
    assert_eq!(desk.refusal(&[&a, &c]), WHOLE_SPAN);
}

#[test]
fn cuts_that_round_to_different_frames_than_the_whole_span_are_refused() {
    // At 30 fps: 0.05 s rounds to 2 frames, 0.1 s to 3. Two pieces of
    // 0.05 s would end on frame 4 while their span ends on frame 3.
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 10.0);
    desk.span(&a, 0.0, 0.05);
    desk.span(&b, 0.05, 0.05);
    assert_eq!(desk.refusal(&[&a, &b]), FRAME_GRID);

    // A piece starting at 0.02 s (frame 1) for 0.05 s ends on frame 3,
    // but the next one, at 0.07 s, starts on frame 2: they would overlap
    // once drawn, though their seconds only touch.
    desk.span(&a, 0.02, 0.05);
    desk.span(&b, 0.07, 0.05);
    assert_eq!(desk.refusal(&[&a, &b]), FRAME_GRID);

    // Fractional seconds that round the same either way are fine:
    // 0.5 s is 15 frames, 0.51 s rounds to 15, 1.01 s to 30.
    desk.span(&a, 0.0, 0.5);
    desk.span(&b, 0.5, 0.51);
    desk.accepts(&[&a, &b]);
}

#[test]
fn a_merge_may_not_cover_visible_pictures_on_lower_lanes() {
    let mut desk = Desk::new();
    let a = desk.place(1, 0.0);
    let b = desk.place(1, 10.0);
    let below = desk.place(0, 5.0);
    assert_eq!(desk.refusal(&[&a, &b]), COVERED);

    // Hidden, the lower lane draws nothing for the merge to cover.
    let lower = desk.lane(0);
    desk.editor
        .apply(Command::SetTrackFlag {
            track_id: lower.clone(),
            flag: TrackFlag::Visible,
            value: false,
        })
        .expect("the lane is hidden");
    desk.accepts(&[&a, &b]);
    desk.editor
        .apply(Command::SetTrackFlag {
            track_id: lower,
            flag: TrackFlag::Visible,
            value: true,
        })
        .expect("the lane is shown again");
    assert_eq!(desk.refusal(&[&a, &b]), COVERED);

    // Sound below has no picture to cover.
    desk.change(&below, |clip| clip.kind = ClipKind::Audio);
    desk.accepts(&[&a, &b]);

    // A picture below that starts where the span ends is not under it.
    desk.change(&below, |clip| {
        clip.kind = ClipKind::Video;
        clip.start = 20.0;
    });
    desk.accepts(&[&a, &b]);

    // Anything above the merged lane stays above the result.
    desk.place(2, 5.0);
    desk.accepts(&[&a, &b]);
}

#[test]
fn a_transition_at_either_edge_of_the_span_needs_its_other_side() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 10.0);
    let c = desk.place(0, 20.0);
    let dissolve = || {
        Some(Transition {
            id: "cross-fade".to_owned(),
            duration: 0.5,
            params: Default::default(),
        })
    };

    desk.change(&b, |clip| clip.transition_in = dissolve());
    desk.accepts(&[&a, &b]);
    assert_eq!(
        desk.refusal(&[&b, &c]),
        TRANSITION,
        "the first clip's way in comes from a clip left out"
    );

    desk.change(&c, |clip| clip.transition_in = dissolve());
    assert_eq!(
        desk.refusal(&[&a, &b]),
        TRANSITION,
        "the clip after the span comes in from its last clip"
    );
    desk.accepts(&[&a, &b, &c]);
}

#[test]
fn the_first_reason_that_applies_is_the_one_given() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 10.0);
    let c = desk.place(0, 20.0);

    // Repeated and of the wrong kind: the count is checked first.
    desk.change(&b, |clip| clip.kind = ClipKind::Image);
    assert_eq!(desk.refusal(&[&a, &b, &b]), TWO_OR_MORE);
    // Of the wrong kind and overlapping: the kind is checked first.
    desk.span(&b, 5.0, 10.0);
    assert_eq!(desk.refusal(&[&a, &b]), SAME_LANE);
    desk.change(&b, |clip| clip.kind = ClipKind::Video);
    assert_eq!(desk.refusal(&[&a, &b]), OVERLAP);
    desk.span(&b, 10.0, 10.0);

    // Detached sound and a clip left out of the span: detached first.
    desk.editor
        .apply(Command::DetachAudio { clip_id: a.clone() })
        .expect("the sound comes off its picture");
    assert_eq!(desk.refusal(&[&a, &c]), DETACHED);
}

#[test]
fn the_clips_come_back_in_timeline_order_from_any_files_in_any_source_order() {
    let mut desk = Desk::new();
    let other = desk.import("/footage/broll.mp4", Some(10.0), MediaKind::Video);
    let a = desk.place(0, 0.0);
    let b = desk.place_of(&other, 0, 10.0);
    let c = desk.place(0, 20.0);
    desk.change(&a, |clip| clip.source_start = 0.0);
    desk.change(&c, |clip| clip.source_start = 0.0);

    let clips = desk.check(&[&c, &a, &b]).expect("mergeable");
    let order: Vec<&str> = clips.iter().map(|clip| clip.id.as_str()).collect();
    assert_eq!(order, [a.as_str(), b.as_str(), c.as_str()]);
    assert_eq!(
        clips[1],
        desk.clip(&b),
        "the clips are returned as they are"
    );
}

// ── the edit that swaps the clips for the rendered file ──

#[test]
fn the_edit_reads_from_the_wire_by_its_published_names() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 10.0);
    let command: Command = serde_json::from_value(json!({
        "op": "consolidateClips",
        "clipIds": [b, a],
        "item": {
            "path": "/project/media/merged-7/merged.mp4",
            "name": "Merged clips",
            "duration": 20.0,
            "kind": "video",
            "width": 1920,
            "height": 1080,
            "frameRate": 30.0,
            "frameRateFraction": "30/1",
            "videoCodec": "h264",
            "audioCodec": "aac",
            "hasAudio": true,
            "origin": "processed"
        }
    }))
    .expect("an API caller can send the edit");
    let Command::ConsolidateClips { clip_ids, item } = &command else {
        panic!("parsed as {command:?}");
    };
    assert_eq!(clip_ids, &[b.clone(), a.clone()]);
    assert_eq!(item.origin, Some(MediaOrigin::Processed));
    let made = desk.editor.apply(command).expect("merges").created_id;
    assert_eq!(made.as_deref(), Some(a.as_str()));
}

#[test]
fn a_file_that_is_not_a_long_enough_video_is_refused_and_nothing_changes() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 12.0);
    let before = desk.editor.project().clone();

    let mut bad = Vec::new();
    bad.push(rendered(22.0 - 1e-5));
    let mut no_length = rendered(22.0);
    no_length.duration = None;
    bad.push(no_length);
    let mut no_path = rendered(22.0);
    no_path.path = String::new();
    bad.push(no_path);
    for kind in [MediaKind::Audio, MediaKind::Image] {
        let mut wrong = rendered(22.0);
        wrong.kind = kind;
        bad.push(wrong);
    }
    for item in bad {
        let error = desk
            .merge(&[&a, &b], item.clone())
            .expect_err("the file cannot stand in for the clips");
        assert!(
            matches!(error, CommandError::CannotMerge { .. }),
            "{item:?}: got {error:?}"
        );
        assert_eq!(error.to_string(), SHORT_FILE);
        assert_eq!(
            desk.editor.project(),
            &before,
            "{item:?} changed the project"
        );
    }

    // Shorter by less than a microsecond is long enough.
    desk.merge(&[&a, &b], rendered(22.0 - 5e-7))
        .expect("a file a hair short still covers the span");
}

#[test]
fn a_refused_selection_refuses_the_edit_and_nothing_changes() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 10.0);
    let c = desk.place(0, 20.0);
    let before = desk.editor.project().clone();
    for ids in [vec![&a], vec![&a, &c], vec![&a, &a]] {
        assert!(desk.merge(&ids, rendered(30.0)).is_err(), "{ids:?}");
        assert_eq!(desk.editor.project(), &before);
    }
    assert!(desk.editor.can_undo(), "the set-up edits are still there");
    let _ = b;
}

#[test]
fn a_number_that_is_not_finite_is_refused() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 10.0);
    let before = desk.editor.project().clone();

    let mut endless = rendered(20.0);
    endless.duration = Some(f64::NAN);
    let error = desk.merge(&[&a, &b], endless).expect_err("NaN length");
    assert!(matches!(error, CommandError::NotANumber), "got {error:?}");

    let mut racing = rendered(20.0);
    racing.frame_rate = Some(f64::INFINITY);
    let error = desk.merge(&[&a, &b], racing).expect_err("infinite rate");
    assert!(matches!(error, CommandError::NotANumber), "got {error:?}");
    assert_eq!(desk.editor.project(), &before);
}

#[test]
fn the_merged_clip_takes_the_span_and_the_first_id_and_starts_plain() {
    let mut desk = Desk::new();
    let other = desk.import("/footage/broll.mp4", Some(10.0), MediaKind::Video);
    let b = desk.place_of(&other, 0, 12.0);
    let a = desk.place(0, 2.0);
    let neighbour = desk.place(0, 30.0);
    let above = desk.place(2, 5.0);
    let lane = desk.lane(0);

    desk.change(&a, |clip| {
        clip.source_start = 0.5;
        clip.duration = 9.5;
        clip.volume = 0.4;
        clip.scale = 1.3;
        clip.opacity = 0.7;
        clip.rotation = 12.0;
        clip.flip_h = true;
        clip.fade_in = 0.5;
        clip.fade_out = 0.25;
        clip.video_effects.push(AppliedFilter {
            id: "concat.vignette".to_owned(),
            params: Default::default(),
            enabled: true,
            keys: Default::default(),
        });
    });
    desk.change(&b, |clip| {
        clip.source_start = 3.0;
        clip.duration = 7.0;
        clip.muted = Some(true);
        clip.transition_in = Some(Transition {
            id: "cross-fade".to_owned(),
            duration: 0.5,
            params: Default::default(),
        });
    });
    let neighbour_before = desk.clip(&neighbour);
    let above_before = desk.clip(&above);
    let bin_before = desk.editor.project().media.clone();

    // Span: 2.0 to 19.0, the gap between 11.5 and 12.0 included.
    let made = desk
        .merge(&[&b, &a], rendered(17.2))
        .expect("the rendered file replaces the clips");
    assert_eq!(
        made.as_deref(),
        Some(a.as_str()),
        "the earliest clip's id is kept"
    );
    assert!(!desk.has(&b), "the later clip is gone");

    let media = desk.editor.project().media.clone();
    assert_eq!(media.len(), bin_before.len() + 1, "one new file in the bin");
    assert_eq!(
        &media[..bin_before.len()],
        &bin_before[..],
        "the originals stay"
    );
    let new = media.last().expect("the rendered file");
    assert_eq!(new.path, "/project/media/merged-1/merged.mp4");
    assert_eq!(new.name, "Merged clips");
    assert_eq!(new.origin, Some(MediaOrigin::Processed));
    assert_eq!(new.kind, MediaKind::Video);

    let merged = desk.clip(&a);
    assert!((merged.start - 2.0).abs() < CLOSE, "start {}", merged.start);
    assert!(
        (merged.duration - 17.0).abs() < CLOSE,
        "the span, not the file's 17.2 s: {}",
        merged.duration
    );
    let mut plain = Clip::blank(
        a.clone(),
        lane,
        ClipKind::Video,
        "Merged clips",
        merged.start,
        merged.duration,
    );
    plain.media_id = new.id.clone();
    assert_eq!(merged, plain, "everything baked into the file is reset");

    assert_eq!(desk.clip(&neighbour), neighbour_before, "nothing ripples");
    assert_eq!(desk.clip(&above), above_before, "other lanes are untouched");
    assert_eq!(desk.editor.project().active().clips.len(), 3);
}

#[test]
fn one_undo_brings_every_clip_and_the_bin_back_and_redo_merges_again() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 10.0);
    let c = desk.place(0, 20.0);
    desk.change(&b, |clip| clip.volume = 0.3);
    let before = desk.editor.project().clone();

    desk.merge(&[&a, &b, &c], rendered(30.0)).expect("merges");
    let after = desk.editor.project().clone();
    assert_eq!(after.active().clips.len(), 1);

    assert!(desk.editor.undo());
    assert_eq!(
        desk.editor.project(),
        &before,
        "one step undoes the whole merge"
    );
    assert!(desk.editor.redo());
    assert_eq!(desk.editor.project(), &after);
    assert!(!desk.editor.can_redo());
}

#[test]
fn inside_a_batch_the_merge_lands_with_the_rest_or_not_at_all() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 10.0);
    let before = desk.editor.project().clone();
    let merge = Command::ConsolidateClips {
        clip_ids: vec![a.clone(), b.clone()],
        item: rendered(20.0),
    };

    // The second merge names clips the first one removed.
    let refused = desk.editor.apply(Command::Batch {
        commands: vec![merge.clone(), merge.clone()],
    });
    assert!(refused.is_err());
    assert_eq!(desk.editor.project(), &before);

    desk.editor
        .apply(Command::Batch {
            commands: vec![
                merge,
                Command::UpdateClip {
                    clip_id: a.clone(),
                    patch: ClipPatch {
                        volume: Some(0.5),
                        ..Default::default()
                    },
                },
            ],
        })
        .expect("the batch applies");
    assert_eq!(desk.clip(&a).volume, 0.5);
    assert!(!desk.has(&b));
    assert!(desk.editor.undo());
    assert_eq!(desk.editor.project(), &before, "the batch is one step");
}

#[test]
fn a_merged_project_saves_and_opens_unchanged() {
    let mut desk = Desk::new();
    let a = desk.place(0, 0.0);
    let b = desk.place(0, 11.0);
    desk.merge(&[&a, &b], rendered(21.0)).expect("merges");

    let document = desk.editor.to_document(&document_settings());
    let reopened = Editor::from_document(&document).expect("the document reads");
    assert_eq!(reopened.project(), desk.editor.project());
    let media = reopened
        .project()
        .media_by_id(
            &reopened
                .project()
                .active()
                .clip(&a)
                .expect("merged")
                .media_id,
        )
        .expect("the merged clip's file is in the bin");
    assert_eq!(media.origin, Some(MediaOrigin::Processed));
}

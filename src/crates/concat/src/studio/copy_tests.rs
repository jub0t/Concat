// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How a copy of a media clip comes out, as Duplicate and Paste make it:
//! the clip's file is laid past the last clip on the lane it is going to,
//! then shaped by one batch of edits into a copy of the source at the
//! asked-for start. These tests drive that on an editor alone, with no
//! window.

use concat_project::commands::NewMedia;
use concat_project::model::{
    AppliedFilter, Clip, ClipKey, KeyProperty, Mask, MediaKind, SpeedPoint, Transition,
};
use concat_project::{Command, Editor};

use super::{TrimEdge, lay_command, shape_copy};

const CLOSE: f64 = 1e-6;

struct Bench {
    editor: Editor,
    media: String,
    track: String,
}

fn file(kind: MediaKind, seconds: Option<f64>) -> NewMedia {
    let picture = kind != MediaKind::Audio;
    NewMedia {
        path: "/footage/take.mov".to_owned(),
        name: "take.mov".to_owned(),
        duration: seconds,
        kind,
        width: picture.then_some(1280),
        height: picture.then_some(720),
        frame_rate: (kind == MediaKind::Video).then_some(25.0),
        frame_rate_fraction: (kind == MediaKind::Video).then(|| "25/1".to_owned()),
        video_codec: picture.then(|| "h264".to_owned()),
        audio_codec: (kind == MediaKind::Audio).then(|| "aac".to_owned()),
        has_audio: kind == MediaKind::Audio,
        audio_tracks: Vec::new(),
        origin: None,
        color_space: Default::default(),
    }
}

impl Bench {
    fn new(kind: MediaKind, seconds: Option<f64>) -> Self {
        let mut editor = Editor::new();
        let media = editor
            .apply(Command::AddMedia {
                item: file(kind, seconds),
            })
            .expect("the file joins the bin")
            .created_id
            .expect("a media id");
        let track = editor.project().active().tracks[0].id.clone();
        Bench {
            editor,
            media,
            track,
        }
    }

    fn run(&mut self, command: Command) -> Option<String> {
        self.editor
            .apply(command)
            .expect("the edit applies")
            .created_id
    }

    fn clip(&self, id: &str) -> Clip {
        self.editor
            .project()
            .active()
            .clip(id)
            .expect("the clip is on the timeline")
            .clone()
    }

    fn clips(&self) -> Vec<Clip> {
        self.editor
            .project()
            .active()
            .clips
            .iter()
            .map(|clip| Clip::clone(clip))
            .collect()
    }

    /// The whole file laid at `start` on the first lane.
    fn lay(&mut self, start: f64) -> String {
        let command = Command::AddClip {
            media_id: self.media.clone(),
            track_id: self.track.clone(),
            start,
            ripple: false,
        };
        self.run(command).expect("a clip id")
    }

    fn trim(&mut self, id: &str, edge: TrimEdge, delta: f64) {
        self.run(Command::TrimClip {
            clip_id: id.to_owned(),
            edge,
            delta,
            ripple: false,
        });
    }

    fn speed(&mut self, id: &str, speed: f64) {
        self.run(Command::SetClipSpeed {
            clip_id: id.to_owned(),
            speed,
            ripple: false,
        });
    }

    /// A new, empty lane beside the first.
    fn other_lane(&mut self) -> String {
        self.run(Command::AddTrack { bottom: false });
        self.editor
            .project()
            .active()
            .tracks
            .iter()
            .map(|track| track.id.clone())
            .find(|id| *id != self.track)
            .expect("a second lane")
    }

    /// Cuts `id` at `time` and says the two pieces, head first.
    fn split(&mut self, id: &str, time: f64) -> (Clip, Clip) {
        let before: Vec<String> = self.clips().into_iter().map(|clip| clip.id).collect();
        self.run(Command::SplitClips {
            clip_ids: vec![id.to_owned()],
            time,
        });
        let head = self.clip(id);
        let tail = self
            .clips()
            .into_iter()
            .find(|clip| !before.contains(&clip.id))
            .expect("the cut makes a second piece");
        (head, tail)
    }

    /// A copy of `source` on `track` at `start`, made the way Duplicate and
    /// Paste make one: the file past the lane's last clip, then one batch.
    fn copy(&mut self, source: &Clip, track: &str, start: f64) -> String {
        let lane_end = self
            .clips()
            .iter()
            .filter(|clip| clip.track_id == track)
            .map(|clip| clip.start + clip.duration)
            .fold(0.0, f64::max);
        let laid = self
            .run(Command::AddClip {
                media_id: source.media_id.clone(),
                track_id: track.to_owned(),
                start: lane_end,
                ripple: false,
            })
            .expect("the file is laid");
        let placed = self.clip(&laid);
        let commands = shape_copy(source, &placed, track, start);
        self.run(Command::Batch { commands });
        laid
    }
}

fn near(left: f64, right: f64) -> bool {
    (left - right).abs() < CLOSE
}

/// The frames a clip shows and the time it takes to show them.
fn assert_same_footage(copy: &Clip, source: &Clip) {
    assert!(
        near(copy.source_start, source.source_start),
        "in-point {} for {}",
        copy.source_start,
        source.source_start
    );
    assert!(
        near(copy.duration, source.duration),
        "length {} for {}",
        copy.duration,
        source.duration
    );
    assert!(
        near(copy.speed, source.speed),
        "speed {} for {}",
        copy.speed,
        source.speed
    );
    assert_eq!(copy.speed_curve, source.speed_curve);
}

/// The second piece of a cut clip, copied right after itself, shows that
/// piece's frames at its speed, not the start of the file; both pieces
/// stay as they were.
#[test]
fn a_copy_of_the_second_piece_of_a_cut_shows_that_piece() {
    for speed in [1.0, 2.0, 0.5] {
        let mut bench = Bench::new(MediaKind::Video, Some(10.0));
        let whole = bench.lay(0.0);
        bench.speed(&whole, speed);
        let (head, tail) = bench.split(&whole, 2.0);
        let end = tail.start + tail.duration;

        let track = bench.track.clone();
        let copy = bench.copy(&tail, &track, end);

        let copy = bench.clip(&copy);
        assert!(
            near(copy.start, end),
            "at speed {speed}: starts {}",
            copy.start
        );
        assert_eq!(copy.track_id, tail.track_id);
        assert_same_footage(&copy, &tail);
        assert_eq!(bench.clip(&head.id), head, "at speed {speed}");
        assert_eq!(bench.clip(&tail.id), tail, "at speed {speed}");
        assert_eq!(bench.clips().len(), 3, "at speed {speed}");
    }
}

/// The first piece of a cut starts at the file's first frame, and its copy
/// does too, with the piece's length.
#[test]
fn a_copy_of_the_first_piece_starts_at_the_first_frame() {
    let mut bench = Bench::new(MediaKind::Video, Some(10.0));
    let whole = bench.lay(0.0);
    let (head, tail) = bench.split(&whole, 4.0);
    let end = tail.start + tail.duration;

    let track = bench.track.clone();
    let copy = bench.copy(&head, &track, end);

    let copy = bench.clip(&copy);
    assert!(near(copy.start, end));
    assert!(near(copy.source_start, 0.0));
    assert_same_footage(&copy, &head);
    assert_eq!(bench.clip(&tail.id), tail);
}

/// A clip trimmed at its head and lying late on the lane, copied to the
/// start of the timeline, keeps its in-point: landing earlier on the
/// timeline than the in-point is into the file changes nothing.
#[test]
fn a_copy_laid_before_its_in_point_keeps_the_in_point() {
    let mut bench = Bench::new(MediaKind::Video, Some(10.0));
    let late = bench.lay(20.0);
    bench.trim(&late, TrimEdge::Start, 6.0);
    let source = bench.clip(&late);
    assert!(near(source.source_start, 6.0) && near(source.duration, 4.0));

    let track = bench.track.clone();
    let copy = bench.copy(&source, &track, 0.0);

    let copy = bench.clip(&copy);
    assert!(near(copy.start, 0.0));
    assert_same_footage(&copy, &source);
    assert_eq!(bench.clip(&late), source);
}

/// A piece cut from inside a clip retimed by a speed curve copies with the
/// curve, the in-point and the length that piece has.
#[test]
fn a_copy_keeps_the_speed_curve() {
    let mut bench = Bench::new(MediaKind::Video, Some(10.0));
    let whole = bench.lay(0.0);
    let point = |at, speed| SpeedPoint {
        at,
        speed,
        slope: None,
    };
    bench.run(Command::SetClipSpeedCurve {
        clip_id: whole.clone(),
        curve: Some(vec![point(0.0, 0.5), point(1.0, 2.0)]),
        ripple: false,
    });
    let curved = bench.clip(&whole);
    assert!(curved.speed_curve.is_some());
    let cut = curved.start + curved.duration * 0.4;
    let (_, tail) = bench.split(&whole, cut);
    let end = tail.start + tail.duration;

    let track = bench.track.clone();
    let copy = bench.copy(&tail, &track, end + 1.0);

    let copy = bench.clip(&copy);
    assert!(near(copy.start, end + 1.0));
    assert_same_footage(&copy, &tail);
    assert_eq!(bench.clip(&tail.id), tail);
}

/// Everything else the source has comes with it - mix, fades, transform,
/// keys, looks, masks, effects - and the copy says it is one by its name.
/// Two things stay behind: the transition on the cut into the source, and
/// the tie to a video its sound was detached from.
#[test]
fn a_copy_carries_everything_but_its_place_and_ties() {
    let mut bench = Bench::new(MediaKind::Video, Some(10.0));
    let laid = bench.lay(0.0);
    bench.trim(&laid, TrimEdge::Start, 1.5);
    bench.trim(&laid, TrimEdge::End, -2.0);
    bench.speed(&laid, 1.25);

    let mut dressed = bench.clip(&laid);
    dressed.volume = 0.4;
    dressed.fade_in = 0.5;
    dressed.fade_out = 0.75;
    dressed.scale = 1.3;
    dressed.offset_x = 0.1;
    dressed.offset_y = -0.2;
    dressed.rotation = 15.0;
    dressed.opacity = 0.8;
    dressed.flip_h = true;
    dressed.blend = "screen".to_owned();
    dressed.preserve_pitch = true;
    dressed.muted = Some(true);
    dressed.keys = vec![
        ClipKey {
            property: KeyProperty::Scale,
            at: 0.0,
            value: 1.0,
            ease: Default::default(),
        },
        ClipKey {
            property: KeyProperty::Scale,
            at: 1.0,
            value: 2.0,
            ease: Default::default(),
        },
    ];
    dressed.video_effects = vec![AppliedFilter::new("concat.blur")];
    dressed.filters = vec![AppliedFilter::new("concat.echo")];
    let mask: Mask =
        serde_json::from_value(serde_json::json!({ "id": "m1", "x": 0.25 })).expect("a mask");
    dressed.masks = vec![mask];
    bench.run(Command::MatchClip {
        clip_id: laid.clone(),
        like: Box::new(dressed),
    });
    let mut source = bench.clip(&laid);
    // Ties the copy does not take: they belong to where the source sits.
    source.transition_in = Some(Transition {
        id: "cross-fade".to_owned(),
        duration: 0.5,
        params: Default::default(),
    });
    source.detached_from = Some("elsewhere".to_owned());

    let track = bench.track.clone();
    let end = source.start + source.duration;
    let copy = bench.copy(&source, &track, end);

    let copy = bench.clip(&copy);
    assert_ne!(copy.id, source.id);
    let mut expected = source.clone();
    expected.id = copy.id.clone();
    expected.start = end;
    expected.name = format!("{} copy", source.name);
    expected.transition_in = None;
    expected.detached_from = None;
    assert_same_footage(&copy, &source);
    // Lengths are compared to the tolerance above; the rest exactly.
    expected.duration = copy.duration;
    expected.source_start = copy.source_start;
    expected.speed = copy.speed;
    assert_eq!(copy, expected);
    assert_eq!(copy.name, "take.mov copy");

    // A copy of a copy says so again.
    let again = bench.copy(&copy, &track, end + copy.duration);
    assert_eq!(bench.clip(&again).name, "take.mov copy copy");
}

/// A start that would cover a clip already there is not where the copy
/// goes: it takes the nearest gap that holds it, and the clips there are
/// neither moved nor trimmed.
#[test]
fn a_copy_onto_a_taken_spot_takes_the_nearest_gap() {
    let mut bench = Bench::new(MediaKind::Video, Some(10.0));
    let first = bench.lay(0.0);
    bench.trim(&first, TrimEdge::End, -7.0);
    let second = bench.lay(5.0);
    bench.trim(&second, TrimEdge::End, -7.0);
    let first = bench.clip(&first);
    let second = bench.clip(&second);
    assert!(near(second.start, 5.0) && near(second.duration, 3.0));

    let track = bench.track.clone();
    let copy = bench.copy(&first, &track, 4.0);

    let copy = bench.clip(&copy);
    // 3..5 is too short for three seconds; after the second clip is the
    // nearest room.
    assert!(near(copy.start, 8.0), "starts {}", copy.start);
    assert_same_footage(&copy, &first);
    assert_eq!(bench.clip(&first.id), first);
    assert_eq!(bench.clip(&second.id), second);
}

/// A copy can go to another lane, an empty one, at the asked-for start;
/// the source's lane is left alone.
#[test]
fn a_copy_lands_on_the_lane_asked_for() {
    let mut bench = Bench::new(MediaKind::Video, Some(10.0));
    let whole = bench.lay(0.0);
    let (_, tail) = bench.split(&whole, 3.0);
    let other = bench.other_lane();

    let copy = bench.copy(&tail, &other, 1.0);

    let copy = bench.clip(&copy);
    assert_eq!(copy.track_id, other);
    assert!(near(copy.start, 1.0));
    assert_same_footage(&copy, &tail);
    let on_source_lane = bench
        .clips()
        .iter()
        .filter(|clip| clip.track_id == bench.track)
        .count();
    assert_eq!(on_source_lane, 2);
}

/// A start before the timeline's beginning is the beginning.
#[test]
fn a_copy_never_starts_before_zero() {
    let mut bench = Bench::new(MediaKind::Video, Some(10.0));
    let whole = bench.lay(0.0);
    let (_, tail) = bench.split(&whole, 6.0);
    let other = bench.other_lane();

    let copy = bench.copy(&tail, &other, -3.0);

    let copy = bench.clip(&copy);
    assert!(near(copy.start, 0.0), "starts {}", copy.start);
    assert_same_footage(&copy, &tail);
}

/// Sound alone copies the same way: the piece's in-point and length.
#[test]
fn a_copy_of_a_sound_piece_shows_that_piece() {
    let mut bench = Bench::new(MediaKind::Audio, Some(30.0));
    let whole = bench.lay(0.0);
    bench.speed(&whole, 1.5);
    let (head, tail) = bench.split(&whole, 7.0);
    let end = tail.start + tail.duration;

    let track = bench.track.clone();
    let copy = bench.copy(&tail, &track, end);

    let copy = bench.clip(&copy);
    assert_eq!(copy.kind, tail.kind);
    assert!(near(copy.start, end));
    assert_same_footage(&copy, &tail);
    assert_eq!(bench.clip(&head.id), head);
}

/// A still has no length of its own: a copy of one held longer or shorter
/// than a fresh still is held exactly as long as its source.
#[test]
fn a_copy_of_a_still_is_held_as_long_as_its_source() {
    for change in [7.0, -2.0] {
        let mut bench = Bench::new(MediaKind::Image, None);
        let still = bench.lay(0.0);
        bench.trim(&still, TrimEdge::End, change);
        let source = bench.clip(&still);
        let end = source.start + source.duration;

        let track = bench.track.clone();
        let copy = bench.copy(&source, &track, end);

        let copy = bench.clip(&copy);
        assert!(near(copy.start, end));
        assert_same_footage(&copy, &source);
        assert_eq!(bench.clip(&still), source);
    }
}

/// The shaping is a single edit: one undo leaves the file as it was first
/// laid, past the lane's last clip, and redo brings the copy back.
#[test]
fn the_shaping_of_a_copy_is_one_undo_step() {
    let mut bench = Bench::new(MediaKind::Video, Some(10.0));
    let whole = bench.lay(0.0);
    let (_, tail) = bench.split(&whole, 2.0);
    let track = bench.track.clone();
    let copy = bench.copy(&tail, &track, 12.0);
    let made = bench.clip(&copy);

    assert!(bench.editor.undo());
    let laid = bench.clip(&copy);
    assert!(near(laid.start, 10.0) && near(laid.source_start, 0.0));
    assert!(near(laid.duration, 10.0));
    assert!(bench.editor.redo());
    assert_eq!(bench.clip(&copy), made);
}

/// The file of a copy is first laid where the lane it is going to ends:
/// past its last clip, or at the start of an empty lane.
#[test]
fn the_file_is_first_laid_where_the_lane_ends() {
    let mut bench = Bench::new(MediaKind::Video, Some(10.0));
    let whole = bench.lay(0.0);
    let (_, tail) = bench.split(&whole, 3.0);
    let other = bench.other_lane();
    let timeline = bench.editor.project().active();
    for (lane, end) in [(bench.track.clone(), 10.0), (other, 0.0)] {
        match lay_command(timeline, &tail, &lane) {
            Command::AddClip {
                media_id,
                track_id,
                start,
                ripple,
            } => {
                assert_eq!(media_id, tail.media_id);
                assert_eq!(track_id, lane);
                assert!(near(start, end), "laid at {start} for {end}");
                assert!(!ripple);
            }
            other => panic!("laid by {other:?}"),
        }
    }
}

/// Duplicate lays and shapes every copy as one gesture: one undo takes all
/// of them away and leaves every source as it was, redo brings every copy
/// back, and the next Duplicate is a step of its own.
#[test]
fn a_duplicate_of_several_clips_is_one_undo_step() {
    let mut bench = Bench::new(MediaKind::Video, Some(10.0));
    let whole = bench.lay(0.0);
    let (head, tail) = bench.split(&whole, 4.0);
    let other = bench.other_lane();
    let third = bench
        .run(Command::AddClip {
            media_id: bench.media.clone(),
            track_id: other.clone(),
            start: 2.0,
            ripple: false,
        })
        .expect("a clip id");
    bench.trim(&third, TrimEdge::Start, 1.0);
    let third = bench.clip(&third);
    let before = bench.clips();

    let duplicate = |bench: &mut Bench, sources: &[Clip]| -> Vec<String> {
        let mut made = Vec::new();
        for source in sources {
            let end = source.start + source.duration;
            let lay = lay_command(bench.editor.project().active(), source, &source.track_id);
            let laid = bench
                .editor
                .apply_within(Some("duplicate"), lay)
                .expect("the file is laid")
                .created_id
                .expect("a clip id");
            let placed = bench.clip(&laid);
            let commands = shape_copy(source, &placed, &source.track_id, end);
            bench
                .editor
                .apply_within(Some("duplicate"), Command::Batch { commands })
                .expect("the copy is shaped");
            made.push(laid);
        }
        bench.editor.end_gesture();
        made
    };

    // Latest first, as Duplicate takes them.
    let sources = [tail.clone(), third.clone(), head.clone()];
    let made = duplicate(&mut bench, &sources);
    for (source, id) in sources.iter().zip(&made) {
        let copy = bench.clip(id);
        assert_eq!(copy.track_id, source.track_id);
        assert_same_footage(&copy, source);
    }
    assert!(near(bench.clip(&made[0]).start, 10.0));
    assert!(near(bench.clip(&made[1]).start, 12.0));
    // Nothing on a lane covers anything else.
    let after = bench.clips();
    for (index, one) in after.iter().enumerate() {
        for two in &after[index + 1..] {
            let apart = one.track_id != two.track_id
                || one.start + one.duration <= two.start + CLOSE
                || two.start + two.duration <= one.start + CLOSE;
            assert!(apart, "{} covers {}", one.id, two.id);
        }
    }

    assert!(bench.editor.undo());
    assert_eq!(bench.clips(), before);
    assert!(bench.editor.redo());
    assert_eq!(bench.clips(), after);

    // Another Duplicate after that is undone alone.
    duplicate(&mut bench, std::slice::from_ref(&third));
    assert_eq!(bench.clips().len(), after.len() + 1);
    assert!(bench.editor.undo());
    assert_eq!(bench.clips(), after);
}

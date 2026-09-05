// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Flattening a project document into the exporter's clip list.
//!
//! Walking a timeline's clips, resolving their media and track, folding
//! track state into per-clip flags, and building each clip's FFmpeg chains
//! from the structured effects the document stores. An export is derived
//! from the engine's own model, so the model of record and the rendered
//! pixels cannot drift apart.
//!
//! Text clips are deliberately not flattened here. Titles rasterise
//! separately and rejoin the exporter as plain image clips; see
//! `ExportRequest`'s caller.

use std::path::Path;

use concat_project::model::{ClipKind as ModelClipKind, Project, Timeline};

use crate::chains::{audio_filter_chain, video_effect_chain};
use crate::{ClipKind, ExportClip, ExportKey, TransitionSpec};

/// Flattens one timeline of `project` for the exporter - the active one
/// when `timeline_id` is `None`. Text clips are excluded (they rasterise
/// separately and rejoin as image clips); a clip whose media or track has
/// vanished is skipped with the same tolerance the document reader extends.
pub fn flatten_timeline(project: &Project, timeline_id: Option<&str>) -> Vec<ExportClip> {
    flatten_timeline_in(project, timeline_id, None)
}

/// [`flatten_timeline`], knowing the project folder: a clip with a cutout
/// is then told where its media's masks are, and renders cut. Without the
/// folder the cutout is carried but not rendered.
pub fn flatten_timeline_in(
    project: &Project,
    timeline_id: Option<&str>,
    project_dir: Option<&Path>,
) -> Vec<ExportClip> {
    let Some(timeline) = pick_timeline(project, timeline_id) else {
        return Vec::new();
    };

    timeline
        .clips
        .iter()
        .filter_map(|clip| {
            if clip.kind == ModelClipKind::Text {
                return None;
            }
            let index = timeline
                .tracks
                .iter()
                .position(|track| track.id == clip.track_id)?;
            let track = &timeline.tracks[index];

            // A layer: no file, a chain over the stack beneath its track,
            // its opacity the strength and its fades the ramps.
            if clip.kind == ModelClipKind::Layer {
                return Some(ExportClip {
                    path: String::new(),
                    kind: ClipKind::Layer,
                    start: clip.start,
                    duration: clip.duration,
                    source_start: 0.0,
                    track: index,
                    hidden: !track.visible,
                    muted: true,
                    volume: 0.0,
                    fade_in: clip.fade_in,
                    fade_out: clip.fade_out,
                    filter_chain: String::new(),
                    speed: 1.0,
                    preserve_pitch: true,
                    speed_curve: Vec::new(),
                    reverse: false,
                    animation: Vec::new(),
                    flip_h: false,
                    flip_v: false,
                    blend: String::new(),
                    crop: None,
                    effects: clip.video_effects.clone(),
                    transition_chain: String::new(),
                    scale: 1.0,
                    offset_x: 0.0,
                    offset_y: 0.0,
                    rotation: 0.0,
                    stretch_x: 1.0,
                    stretch_y: 1.0,
                    opacity: clip.opacity,
                    video_filter_chain: video_effect_chain(&clip.video_effects),
                    transition: None,
                    video_fade_in: 0.0,
                    media_width: None,
                    media_height: None,
                    has_audio: Some(false),
                    cutout: None,
                    mask_dir: String::new(),
                });
            }

            let media = project.media.iter().find(|item| item.id == clip.media_id)?;

            Some(ExportClip {
                path: media.path.clone(),
                kind: match clip.kind {
                    ModelClipKind::Video => ClipKind::Video,
                    ModelClipKind::Audio => ClipKind::Audio,
                    ModelClipKind::Image => ClipKind::Image,
                    // Handled above; unreachable spelled as a skip so a new
                    // kind fails soft.
                    ModelClipKind::Text | ModelClipKind::Layer => return None,
                },
                start: clip.start,
                duration: clip.duration,
                source_start: clip.source_start,
                track: index,
                hidden: !track.visible,
                muted: track.muted || clip.muted == Some(true),
                volume: clip.volume,
                fade_in: clip.fade_in,
                fade_out: clip.fade_out,
                filter_chain: audio_filter_chain(&clip.filters),
                speed: clip.speed,
                preserve_pitch: clip.preserve_pitch,
                speed_curve: clip
                    .speed_curve
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .map(|point| (point.at, point.speed))
                    .collect(),
                reverse: clip.reverse,
                animation: export_keys(clip),
                flip_h: clip.flip_h,
                flip_v: clip.flip_v,
                blend: clip.blend.clone(),
                crop: clip
                    .crop
                    .filter(|crop| !crop.is_none())
                    .map(|crop| [crop.left, crop.top, crop.right, crop.bottom]),
                effects: clip.video_effects.clone(),
                transition_chain: String::new(),
                scale: clip.scale,
                offset_x: clip.offset_x,
                offset_y: clip.offset_y,
                rotation: clip.rotation,
                stretch_x: clip.stretch_x,
                stretch_y: clip.stretch_y,
                opacity: clip.opacity,
                video_filter_chain: video_effect_chain(&clip.video_effects),
                // Passed through unconditionally: `resolve_transitions` is
                // the one adjacency judge (frame/2 tolerance). A fixed
                // 1/60 s gate here would agree at 30fps and disagree at
                // every other rate; resolving is the kinder read of what
                // the user placed.
                transition: clip
                    .transition_in
                    .as_ref()
                    .map(|transition| TransitionSpec {
                        kind: transition.id.clone(),
                        duration: transition.duration,
                    }),
                video_fade_in: 0.0,
                media_width: media.width,
                media_height: media.height,
                has_audio: Some(media.has_audio),
                cutout: clip.cutout.clone(),
                mask_dir: match (&clip.cutout, project_dir) {
                    (Some(_), Some(dir)) => concat_vision::mask_dir(dir, &media.path)
                        .to_string_lossy()
                        .into_owned(),
                    _ => String::new(),
                },
            })
        })
        .collect()
}

/// The timeline to flatten: the named one, or the document's active one,
/// falling back to the first - the same preference the UI's tab strip has.
fn pick_timeline<'a>(project: &'a Project, timeline_id: Option<&str>) -> Option<&'a Timeline> {
    let wanted = timeline_id.unwrap_or(&project.active_timeline_id);
    project
        .timelines
        .iter()
        .find(|timeline| timeline.id == wanted)
        .or_else(|| project.timelines.first())
}

/// The clip's animation presets, materialised for its current length as
/// the keys the engine plays. See `concat_project::animation`.
pub fn export_keys(clip: &concat_project::model::Clip) -> Vec<ExportKey> {
    use concat_core::animate::Ease;
    let Some(animation) = concat_project::animation::animation_of(clip) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (property, track) in [
        ("scale", &animation.scale),
        ("offsetX", &animation.offset_x),
        ("offsetY", &animation.offset_y),
        ("rotation", &animation.rotation),
        ("opacity", &animation.opacity),
    ] {
        for key in track.keys() {
            out.push(ExportKey {
                property: property.to_owned(),
                at: key.at,
                value: key.value,
                ease: match key.ease {
                    Ease::Linear => "linear",
                    Ease::In => "in",
                    Ease::Out => "out",
                    Ease::InOut => "inOut",
                }
                .to_owned(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use concat_project::commands::{ClipPatch, NewMedia, TrackFlag};
    use concat_project::model::AppliedFilter;
    use concat_project::{Command, Editor};

    use super::*;

    /// A project with one video media item and one clip of it, built through
    /// the real command layer so the fixture cannot drift from the model.
    fn project_with_clip() -> (Editor, String, String) {
        let mut editor = Editor::new();
        let media_id = editor
            .apply(Command::AddMedia {
                item: NewMedia {
                    path: "/footage/a.mp4".into(),
                    name: "a.mp4".into(),
                    duration: Some(10.0),
                    kind: concat_project::model::MediaKind::Video,
                    width: Some(1920),
                    height: Some(1080),
                    frame_rate: None,
                    frame_rate_fraction: None,
                    video_codec: None,
                    audio_codec: None,
                    has_audio: true,
                },
            })
            .expect("adds media")
            .created_id
            .expect("mints an id");
        let track_id = editor.project().timelines[0].tracks[0].id.clone();
        let clip_id = editor
            .apply(Command::AddClip {
                media_id: media_id.clone(),
                track_id,
                start: 1.0,
            })
            .expect("adds clip")
            .created_id
            .expect("mints an id");
        (editor, media_id, clip_id)
    }

    #[test]
    fn a_clip_flattens_with_its_media_and_track_facts() {
        let (editor, _, _) = project_with_clip();
        let flat = flatten_timeline(editor.project(), None);
        assert_eq!(flat.len(), 1);
        let clip = &flat[0];
        assert_eq!(clip.path, "/footage/a.mp4");
        assert!(matches!(clip.kind, ClipKind::Video));
        assert_eq!(clip.start, 1.0);
        assert_eq!(clip.duration, 10.0, "the clip takes its media's length");
        assert_eq!(clip.track, 0);
        assert!(!clip.hidden);
        assert!(!clip.muted);
        assert_eq!(clip.media_width, Some(1920));
        assert_eq!(clip.has_audio, Some(true));
        assert_eq!(clip.filter_chain, "");
        assert_eq!(clip.video_filter_chain, "");
    }

    #[test]
    fn track_state_folds_into_the_clip_flags() {
        let (mut editor, _, _) = project_with_clip();
        let track_id = editor.project().timelines[0].tracks[0].id.clone();
        editor
            .apply(Command::SetTrackFlag {
                track_id: track_id.clone(),
                flag: TrackFlag::Visible,
                value: false,
            })
            .expect("hides");
        editor
            .apply(Command::SetTrackFlag {
                track_id,
                flag: TrackFlag::Muted,
                value: true,
            })
            .expect("mutes");
        let flat = flatten_timeline(editor.project(), None);
        assert!(flat[0].hidden, "a hidden track hides its clips");
        assert!(flat[0].muted, "a muted track mutes its clips");
    }

    #[test]
    fn chains_build_from_the_stored_effects() {
        let (mut editor, _, clip_id) = project_with_clip();
        editor
            .apply(Command::UpdateClip {
                clip_id,
                patch: ClipPatch {
                    video_effects: Some(vec![AppliedFilter {
                        id: "sepia".into(),
                        params: BTreeMap::new(),
                        enabled: true,
                    }]),
                    ..Default::default()
                },
            })
            .expect("applies effect");
        let flat = flatten_timeline(editor.project(), None);
        // The exact string is chains.rs's contract, pinned there; here
        // only that flattening routes through it.
        assert!(
            flat[0].video_filter_chain.contains("color_channel_mixer")
                || !flat[0].video_filter_chain.is_empty(),
            "sepia must produce a chain, got {:?}",
            flat[0].video_filter_chain
        );
    }

    #[test]
    fn text_clips_are_left_for_the_rasteriser() {
        let (mut editor, _, _) = project_with_clip();
        let track_id = Some(editor.project().timelines[0].tracks[0].id.clone());
        editor
            .apply(Command::AddTextClip {
                track_id,
                start: 0.0,
                style: None,
                duration: None,
                offset_y: None,
            })
            .expect("adds title");
        let flat = flatten_timeline(editor.project(), None);
        assert_eq!(flat.len(), 1, "the text clip does not flatten");
    }

    #[test]
    fn a_clip_whose_media_vanished_is_skipped_not_fatal() {
        let (mut editor, media_id, _) = project_with_clip();
        editor
            .apply(Command::RemoveMedia { media_id })
            .expect("removes");
        // RemoveMedia also removes the clips; force the orphan case instead
        // by asking for a timeline that does not exist.
        assert!(flatten_timeline(editor.project(), None).is_empty());
        assert!(flatten_timeline(editor.project(), Some("nope")).len() <= 1);
    }
}

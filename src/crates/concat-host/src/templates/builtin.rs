// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The templates that come with the app.
//!
//! Built here, through the engine's own commands, rather than shipped as
//! JSON: a schema change breaks them at compile time instead of leaving a
//! gallery card that will not open. They carry nothing but slots, titles,
//! moves and transitions - no music or pictures of anyone's - so there is
//! nothing to bundle and nothing to license.
//!
//! On every listing they are written out as ordinary bundles under the
//! config directory's `builtin-templates/`, so the rest of the library -
//! the gallery, `instantiate`, the API - treats them like any other.

use std::path::Path;

use concat_project::commands::{ClipPatch, NewMedia};
use concat_project::model::{
    KeyEase, KeyProperty, MediaKind, TextStyle, Transition, VideoSettings,
};
use concat_project::{Command, DocumentSettings, Editor};
use serde_json::Value;

/// Where the built-in bundles are written, under the config directory.
pub fn dir(config: &Path) -> std::path::PathBuf {
    config.join("builtin-templates")
}

/// One built-in: its folder name and its document.
pub struct Builtin {
    /// The bundle folder's name, which is also how the window names it.
    pub id: &'static str,
    /// The template, as `template.json` holds it.
    pub document: Value,
}

/// Every built-in, in the order the gallery shows them.
pub fn all() -> Vec<Builtin> {
    vec![
        Builtin {
            id: "photo-slideshow",
            document: slideshow(),
        },
        Builtin {
            id: "quick-cuts",
            document: quick_cuts(),
        },
        Builtin {
            id: "vlog-intro",
            document: vlog_intro(),
        },
    ]
}

/// Writes every built-in bundle that is missing or out of date, and
/// returns the folders. A bundle that cannot be written is left out of
/// the gallery rather than failing it.
pub fn install(config: &Path) -> Vec<std::path::PathBuf> {
    let root = dir(config);
    let mut folders = Vec::new();
    for builtin in all() {
        let folder = root.join(builtin.id);
        let manifest = folder.join(super::MANIFEST);
        let Ok(encoded) = serde_json::to_vec_pretty(&builtin.document) else {
            continue;
        };
        let current = std::fs::read(&manifest).ok();
        if current.as_deref() != Some(encoded.as_slice())
            && let Err(error) =
                std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(&manifest, &encoded))
        {
            log::warn!("could not write the {} template: {error}", builtin.id);
            continue;
        }
        folders.push(folder);
    }
    folders
}

/// A template under construction: an editor on a fresh timeline, and the
/// slots made so far.
struct Draft {
    editor: Editor,
    name: &'static str,
    video: VideoSettings,
    slots: usize,
}

impl Draft {
    fn new(name: &'static str, width: u32, height: u32) -> Self {
        let video = VideoSettings {
            width,
            height,
            rate_num: 30,
            rate_den: 1,
            ..VideoSettings::default()
        };
        Self {
            editor: Editor::with_video(video),
            name,
            video,
            slots: 0,
        }
    }

    fn apply(&mut self, command: Command) -> Option<String> {
        match self.editor.apply(command) {
            Ok(outcome) => outcome.created_id,
            Err(error) => {
                // A built-in is code; a command it gets wrong is a bug the
                // tests below catch, not something a user can cause.
                debug_assert!(false, "{}: {error}", self.name);
                log::warn!("{}: {error}", self.name);
                None
            }
        }
    }

    /// A slot `seconds` long at `start` on the bottom track, and its clip.
    /// The path is a stand-in only so that `AddMedia`, which folds media
    /// with one path together, keeps each slot its own; it is blanked when
    /// the document is written, as `save` blanks a creator's own footage.
    fn slot(&mut self, start: f64, seconds: f64) -> String {
        self.slots += 1;
        let number = self.slots;
        let media = self
            .apply(Command::AddMedia {
                item: NewMedia {
                    path: format!("slot-{number}"),
                    name: format!("{number}"),
                    duration: Some(seconds),
                    kind: MediaKind::Video,
                    width: Some(self.video.width),
                    height: Some(self.video.height),
                    frame_rate: Some(30.0),
                    frame_rate_fraction: Some("30/1".to_owned()),
                    video_codec: None,
                    audio_codec: None,
                    has_audio: false,
                    audio_tracks: Vec::new(),
                    origin: None,
                    color_space: Default::default(),
                },
            })
            .unwrap_or_default();
        self.apply(Command::SetMediaPlaceholder {
            media_id: media.clone(),
            placeholder: true,
        });
        self.apply(Command::AddClip {
            media_id: media,
            track_id: "T1".to_owned(),
            start,
            ripple: false,
        })
        .unwrap_or_default()
    }

    /// A key on one of a clip's properties, `at` a fraction of the way in.
    fn key(&mut self, clip: &str, property: KeyProperty, at: f64, value: f64) {
        self.apply(Command::SetClipKey {
            clip_id: clip.to_owned(),
            property,
            at,
            value,
            ease: KeyEase::default(),
        });
    }

    /// A transition into `clip` from the one before it on its track.
    fn transition(&mut self, clip: &str, id: &str, seconds: f64) {
        self.apply(Command::UpdateClip {
            clip_id: clip.to_owned(),
            patch: ClipPatch {
                transition_in: Some(Some(Transition {
                    id: id.to_owned(),
                    duration: seconds,
                })),
                ..ClipPatch::default()
            },
        });
    }

    /// A title over the picture.
    fn title(&mut self, start: f64, seconds: f64, offset_y: f64, style: TextStyle) -> String {
        self.apply(Command::AddTextClip {
            track_id: None,
            above: true,
            start,
            style: Some(style),
            duration: Some(seconds),
            offset_y: Some(offset_y),
        })
        .unwrap_or_default()
    }

    fn finish(self) -> Value {
        let mut project = self.editor.project().clone();
        for item in &mut project.media {
            if item.placeholder {
                item.path = String::new();
            }
        }
        let settings = DocumentSettings {
            name: self.name.to_owned(),
            width: self.video.width,
            height: self.video.height,
            rate_num: self.video.rate_num,
            rate_den: self.video.rate_den,
        };
        concat_project::to_document(&settings, &project)
    }
}

fn words(content: &str, size: f64) -> TextStyle {
    TextStyle {
        content: content.to_owned(),
        font_size: size,
        ..TextStyle::default()
    }
}

/// Six pictures, each drifting in or out, dissolving into the next, under
/// a title for the first of them. Upright, for a phone's feed.
fn slideshow() -> Value {
    let mut draft = Draft::new("Photo slideshow", 1080, 1920);
    let length = 2.5;
    for index in 0..6 {
        let clip = draft.slot(index as f64 * length, length);
        // Alternate the drift so a run of stills never feels like one
        // long zoom.
        let (from, to) = if index % 2 == 0 {
            (1.0, 1.15)
        } else {
            (1.15, 1.0)
        };
        draft.key(&clip, KeyProperty::Scale, 0.0, from);
        draft.key(&clip, KeyProperty::Scale, 1.0, to);
        if index > 0 {
            draft.transition(&clip, "concat.dissolve", 0.6);
        }
    }
    let title = draft.title(0.2, 2.1, 0.3, words("Our memories", 0.07));
    draft.key(&title, KeyProperty::Opacity, 0.0, 0.0);
    draft.key(&title, KeyProperty::Opacity, 0.15, 1.0);
    draft.key(&title, KeyProperty::Opacity, 0.85, 1.0);
    draft.key(&title, KeyProperty::Opacity, 1.0, 0.0);
    draft.finish()
}

/// Ten short cuts that punch in on every change, with zooms between every
/// other pair, and a word over the opening. Upright.
fn quick_cuts() -> Value {
    let mut draft = Draft::new("Quick cuts", 1080, 1920);
    let length = 0.6;
    for index in 0..10 {
        let clip = draft.slot(index as f64 * length, length);
        draft.key(&clip, KeyProperty::Scale, 0.0, 1.2);
        draft.key(&clip, KeyProperty::Scale, 0.5, 1.0);
        if index > 0 && index % 2 == 0 {
            draft.transition(&clip, "concat.zoom-transition", 0.2);
        }
    }
    let mut style = words("LET'S GO", 0.1);
    style.font_weight = 900.0;
    style.stroke_width = 0.006;
    draft.title(0.0, 1.2, 0.0, style);
    draft.finish()
}

/// An opening shot under the vlog's name, three quick ones whipped
/// together, and a closing shot with a goodbye. Wide, for a channel.
fn vlog_intro() -> Value {
    let mut draft = Draft::new("Vlog intro", 1920, 1080);
    let opening = draft.slot(0.0, 4.0);
    draft.key(&opening, KeyProperty::Scale, 0.0, 1.0);
    draft.key(&opening, KeyProperty::Scale, 1.0, 1.08);
    let mut start = 4.0;
    for _ in 0..3 {
        let clip = draft.slot(start, 2.0);
        draft.transition(&clip, "concat.whip-pan", 0.4);
        start += 2.0;
    }
    let closing = draft.slot(start, 3.0);
    draft.transition(&closing, "concat.dissolve", 0.6);

    let name = draft.title(0.4, 3.2, -0.05, words("MY VLOG", 0.12));
    draft.key(&name, KeyProperty::Scale, 0.0, 0.85);
    draft.key(&name, KeyProperty::Scale, 0.2, 1.0);
    draft.key(&name, KeyProperty::Opacity, 0.0, 0.0);
    draft.key(&name, KeyProperty::Opacity, 0.2, 1.0);
    draft.key(&name, KeyProperty::Opacity, 0.85, 1.0);
    draft.key(&name, KeyProperty::Opacity, 1.0, 0.0);
    let mut episode = words("Episode 1", 0.05);
    episode.font_weight = 500.0;
    draft.title(0.8, 2.8, 0.1, episode);

    draft.title(start + 0.5, 2.3, 0.0, words("Thanks for watching!", 0.08));
    draft.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every built-in reads back as a template: a format, and slots in
    /// timeline order with no path behind any of them.
    #[test]
    fn every_builtin_is_a_template() {
        for builtin in all() {
            let project = concat_project::from_document(&builtin.document)
                .unwrap_or_else(|| panic!("{} reads back", builtin.id));
            let slots: Vec<_> = project.media.iter().filter(|m| m.placeholder).collect();
            assert!(!slots.is_empty(), "{} has slots", builtin.id);
            assert!(
                slots.iter().all(|slot| slot.path.is_empty()),
                "{}: slots carry no path",
                builtin.id
            );
            assert!(
                project.media.iter().all(|item| item.placeholder),
                "{}: nothing to bundle",
                builtin.id
            );
            let clips = &project.active().clips;
            for slot in &slots {
                assert!(
                    clips.iter().any(|clip| clip.media_id == slot.id),
                    "{}: slot {} is on the timeline",
                    builtin.id,
                    slot.name
                );
            }
            assert!(
                clips.iter().any(|clip| clip.text.is_some()),
                "{} has a title",
                builtin.id
            );
        }
    }

    /// The transitions landed: each built-in asked for some, and a
    /// transition the engine refused would leave none.
    #[test]
    fn transitions_land_between_slots() {
        for builtin in all() {
            let project = concat_project::from_document(&builtin.document).expect("reads");
            let count = project
                .active()
                .clips
                .iter()
                .filter(|clip| clip.transition_in.is_some())
                .count();
            assert!(count > 0, "{} has transitions", builtin.id);
        }
    }

    /// Writing them out twice changes nothing the second time, and what
    /// was written lists as a template with every slot.
    #[test]
    fn installing_is_idempotent_and_lists() {
        let scratch = std::env::temp_dir().join(format!("concat-builtins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let first = install(&scratch);
        assert_eq!(first.len(), all().len());
        let stamp = std::fs::metadata(first[0].join(super::super::MANIFEST))
            .and_then(|meta| meta.modified())
            .expect("written");
        let second = install(&scratch);
        assert_eq!(first, second);
        let again = std::fs::metadata(second[0].join(super::super::MANIFEST))
            .and_then(|meta| meta.modified())
            .expect("still there");
        assert_eq!(stamp, again, "an unchanged bundle is not rewritten");

        let listed = super::super::list(&scratch);
        assert_eq!(listed.len(), all().len());
        assert!(listed.iter().all(|info| info.builtin));
        let slideshow = listed
            .iter()
            .find(|info| info.id == "photo-slideshow")
            .expect("listed");
        assert_eq!(slideshow.slots.len(), 6);
        assert_eq!((slideshow.width, slideshow.height), (1080, 1920));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Each built-in fills: every slot takes a picture, and the project it
    /// makes has no slot left in it and keeps its titles and transitions.
    #[test]
    fn every_builtin_fills_into_a_project() {
        let scratch =
            std::env::temp_dir().join(format!("concat-builtin-fill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let config = scratch.join("config");
        let location = scratch.join("projects");
        std::fs::create_dir_all(&location).expect("scratch dir");
        for info in super::super::list(&config) {
            let fills = info
                .slots
                .iter()
                .enumerate()
                .map(|(index, slot)| super::super::SlotFill {
                    media_id: slot.media_id.clone(),
                    item: NewMedia {
                        path: format!("/mine/photo-{index}.jpg"),
                        name: format!("photo-{index}.jpg"),
                        duration: None,
                        kind: MediaKind::Image,
                        width: Some(3000),
                        height: Some(2000),
                        frame_rate: None,
                        frame_rate_fraction: None,
                        video_codec: Some("mjpeg".to_owned()),
                        audio_codec: None,
                        has_audio: false,
                        audio_tracks: Vec::new(),
                        origin: None,
                        color_space: Default::default(),
                    },
                })
                .collect();
            let made = super::super::instantiate(
                &info.path,
                &location.to_string_lossy(),
                &info.name,
                fills,
            )
            .unwrap_or_else(|error| panic!("{} fills: {error}", info.id));
            let document = crate::projects::read_document(&made.path)
                .expect("reads back")
                .expect("a document");
            let project = concat_project::from_document(&document).expect("parses");
            assert!(
                project.media.iter().all(|item| !item.placeholder),
                "{}: no slot left",
                info.id
            );
            let clips = &project.active().clips;
            assert!(
                clips.iter().any(|clip| clip.text.is_some()),
                "{}: titles kept",
                info.id
            );
            assert!(
                clips.iter().any(|clip| clip.transition_in.is_some()),
                "{}: transitions kept",
                info.id
            );
            assert!(
                (info.seconds
                    - clips
                        .iter()
                        .map(|c| c.start + c.duration)
                        .fold(0.0, f64::max))
                .abs()
                    < 1e-6,
                "{}: the timing is the template's",
                info.id
            );
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }
}

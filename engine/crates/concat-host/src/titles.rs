// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Titles, rendered and remembered.
//!
//! The flattener leaves text clips out on purpose: the compositor only knows
//! pictures. This is where a title becomes one. Each text clip's style is
//! painted by `concat-text` onto a frame-sized transparent PNG in the app's
//! data directory, and the clip rejoins the flattened list as an image clip
//! pointing at that file, carrying the text clip's timing, transform and
//! opacity. The monitor, playback prefetch and the exporter then treat it as
//! any other still.
//!
//! The file is keyed by everything that changes the pixels - the style, the
//! frame size, the fonts the project carries - so a title that has not
//! changed is never painted twice, across sessions included. What a title
//! does *not* key on is where it sits or when it plays: moving one is free.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use concat_export::chains::video_effect_chain;
use concat_export::{ClipKind, ExportClip};
use concat_project::model::{ClipKind as ModelClipKind, Project, TextAlign, TextStyle};
use concat_text::{Align, Fonts, TitleStyle};

use crate::dirs::AppDirs;

/// One title, ready for the compositor.
#[derive(Clone)]
pub struct TitleClip {
    /// The text clip this was painted from.
    pub clip_id: String,
    /// The image clip that stands in for it.
    pub clip: ExportClip,
    /// The painted block's size in frame pixels, for an outline on a monitor.
    pub block: (u32, u32),
}

/// What one render left behind.
#[derive(Clone, Copy, Debug)]
struct Art {
    block: (u32, u32),
}

/// The title painter and its cache.
pub struct Titles {
    dir: PathBuf,
    /// The system's faces plus the project's files, loaded once each.
    fonts: Mutex<Option<Fonts>>,
    loaded_files: Mutex<HashSet<String>>,
    /// What is known to be on disk, by key.
    memo: Mutex<HashMap<u64, Art>>,
}

impl Titles {
    /// A painter that caches under the app's data directory.
    pub fn new(dirs: &AppDirs) -> Titles {
        Titles {
            dir: dirs.data.join("titles"),
            fonts: Mutex::new(None),
            loaded_files: Mutex::new(HashSet::new()),
            memo: Mutex::new(HashMap::new()),
        }
    }

    /// Every text clip on the active timeline, as an image clip each, for a
    /// `width` × `height` frame. A title that fails to paint is left out and
    /// said once on stderr; the rest of the edit still renders.
    pub fn clips(&self, project: &Project, width: u32, height: u32) -> Vec<TitleClip> {
        let timeline = project.active();
        let mut out = Vec::new();
        for clip in &timeline.clips {
            if clip.kind != ModelClipKind::Text {
                continue;
            }
            let Some(index) = timeline
                .tracks
                .iter()
                .position(|track| track.id == clip.track_id)
            else {
                continue;
            };
            let track = &timeline.tracks[index];
            let text = clip.text.clone().unwrap_or_default();
            let (path, block) = match self.painted(project, &text, width, height) {
                Ok(art) => art,
                Err(error) => {
                    eprintln!("concat: title {}: {error}", clip.id);
                    continue;
                }
            };
            out.push(TitleClip {
                clip_id: clip.id.clone(),
                clip: ExportClip {
                    path: path.to_string_lossy().into_owned(),
                    kind: ClipKind::Image,
                    start: clip.start,
                    duration: clip.duration,
                    source_start: 0.0,
                    track: index,
                    hidden: !track.visible,
                    muted: true,
                    volume: 0.0,
                    fade_in: 0.0,
                    fade_out: 0.0,
                    filter_chain: String::new(),
                    speed: 1.0,
                    preserve_pitch: true,
                    speed_curve: Vec::new(),
                    reverse: false,
                    animation: concat_export::flatten::export_keys(clip),
                    flip_h: clip.flip_h,
                    flip_v: clip.flip_v,
                    blend: clip.blend.clone(),
                    crop: None,
                    effects: clip.video_effects.clone(),
                    transition_chain: String::new(),
                    scale: clip.scale,
                    offset_x: clip.offset_x,
                    offset_y: clip.offset_y,
                    rotation: clip.rotation,
                    // The style's own opacity multiplies the clip's: a
                    // half-transparent title fades to half, not to solid.
                    opacity: (clip.opacity * text.opacity).clamp(0.0, 1.0),
                    video_filter_chain: video_effect_chain(&clip.video_effects),
                    transition: None,
                    video_fade_in: 0.0,
                    media_width: Some(width),
                    media_height: Some(height),
                    has_audio: Some(false),
                },
                block,
            });
        }
        out
    }

    /// The PNG for one style at one frame size, painted if it is not on disk
    /// yet, and the block it holds.
    fn painted(
        &self,
        project: &Project,
        style: &TextStyle,
        width: u32,
        height: u32,
    ) -> Result<(PathBuf, (u32, u32)), String> {
        let key = key_of(project, style, width, height);
        let png = self.dir.join(format!("{key:016x}.png"));
        let side = self.dir.join(format!("{key:016x}.json"));

        if let Some(art) = self
            .memo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            && png.is_file()
        {
            return Ok((png, art.block));
        }
        // On disk from an earlier session: the sidecar says how big the
        // block was, which the PNG alone cannot.
        if png.is_file()
            && let Some(block) = read_block(&side)
        {
            self.remember(key, block);
            return Ok((png, block));
        }

        let title = title_style(style);
        let rendered = {
            let mut fonts = self.fonts.lock().unwrap_or_else(|e| e.into_inner());
            let fonts = fonts.get_or_insert_with(Fonts::new);
            // The project's own files, each loaded once for the process.
            let mut loaded = self.loaded_files.lock().unwrap_or_else(|e| e.into_inner());
            for font in &project.fonts {
                if !font.path.is_empty() && loaded.insert(font.path.clone()) {
                    fonts.add_file(Path::new(&font.path));
                }
            }
            concat_text::render(fonts, &title, width, height).map_err(|error| error.to_string())?
        };
        std::fs::create_dir_all(&self.dir).map_err(|error| error.to_string())?;
        std::fs::write(&png, &rendered.png).map_err(|error| error.to_string())?;
        let block = (rendered.block_width, rendered.block_height);
        let _ = std::fs::write(&side, format!("{{\"w\":{},\"h\":{}}}", block.0, block.1));
        self.remember(key, block);
        Ok((png, block))
    }

    fn remember(&self, key: u64, block: (u32, u32)) {
        self.memo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, Art { block });
    }
}

/// Everything the pixels depend on, hashed. Placement and timing are left
/// out on purpose; see the module docs.
fn key_of(project: &Project, style: &TextStyle, width: u32, height: u32) -> u64 {
    let mut hasher = DefaultHasher::new();
    // The style as JSON: every field, in a stable order, with no need to
    // keep a Hash impl in step with the struct.
    serde_json::to_string(style)
        .unwrap_or_default()
        .hash(&mut hasher);
    width.hash(&mut hasher);
    height.hash(&mut hasher);
    for font in &project.fonts {
        font.family.hash(&mut hasher);
        font.path.hash(&mut hasher);
    }
    // Bumped when the painter's output changes for the same input, so stale
    // files are not mistaken for current ones.
    1u32.hash(&mut hasher);
    hasher.finish()
}

fn read_block(side: &Path) -> Option<(u32, u32)> {
    let text = std::fs::read_to_string(side).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some((
        value.get("w")?.as_u64()? as u32,
        value.get("h")?.as_u64()? as u32,
    ))
}

/// The document's style, field for field, in the painter's terms.
fn title_style(style: &TextStyle) -> TitleStyle {
    TitleStyle {
        content: style.content.clone(),
        font_family: style.font_family.clone(),
        font_size: style.font_size,
        font_weight: style.font_weight,
        italic: style.italic,
        color: style.color.clone(),
        align: match style.align {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        },
        stroke_width: style.stroke_width,
        stroke_color: style.stroke_color.clone(),
        shadow: style.shadow,
        background: style.background.clone(),
        line_height: style.line_height,
        tracking: style.tracking,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_project::{Command, Editor};

    fn scratch() -> AppDirs {
        let dir = std::env::temp_dir().join(format!("concat-titles-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        AppDirs {
            config: dir.join("config"),
            data: dir.join("data"),
        }
    }

    /// A text clip comes back as an image clip on its own track, pointing
    /// at a PNG on disk that is the frame's size, and a second ask for the
    /// same title paints nothing new.
    #[test]
    fn a_title_rejoins_as_a_still() {
        let dirs = scratch();
        let mut editor = Editor::new();
        let id = editor
            .apply(Command::AddTextClip {
                track_id: None,
                start: 2.0,
                style: None,
                duration: Some(3.0),
                offset_y: Some(0.3),
            })
            .expect("a title is added")
            .created_id
            .expect("with an id");

        let titles = Titles::new(&dirs);
        let out = titles.clips(editor.project(), 640, 360);
        assert_eq!(out.len(), 1);
        let title = &out[0];
        assert_eq!(title.clip_id, id);
        assert!(matches!(title.clip.kind, ClipKind::Image));
        assert_eq!((title.clip.start, title.clip.duration), (2.0, 3.0));
        assert_eq!(title.clip.offset_y, 0.3);
        assert_eq!(title.clip.media_width, Some(640));
        assert!(title.block.0 > 0 && title.block.1 > 0);
        let painted = Path::new(&title.clip.path);
        assert!(painted.is_file(), "the PNG is on disk");
        let stamp = std::fs::metadata(painted).and_then(|m| m.modified()).ok();

        // Same words, same frame: same file, untouched.
        let again = titles.clips(editor.project(), 640, 360);
        assert_eq!(again[0].clip.path, title.clip.path);
        assert_eq!(
            std::fs::metadata(painted).and_then(|m| m.modified()).ok(),
            stamp
        );

        // A different frame is a different picture.
        let wide = titles.clips(editor.project(), 1280, 720);
        assert_ne!(wide[0].clip.path, title.clip.path);
        let _ = std::fs::remove_dir_all(dirs.data.parent().unwrap());
    }
}

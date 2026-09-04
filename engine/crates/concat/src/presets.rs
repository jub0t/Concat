// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Text presets: a title's look, named, that the library's Text page
//! offers as a card. The built-in ones ship in the binary; a folder of
//! TOML files beside the app's settings adds more, and a preset shared
//! between people travels as a folder with its font inside.
//!
//! A preset file, `text-presets/<anything>.toml` or
//! `text-presets/<anything>/preset.toml` under the config directory:
//!
//! ```toml
//! id = "studio.big-red"        # unique for ever: project files store it
//! name = "Big Red"
//! font = "BigRed.ttf"          # optional: a file beside this one
//! offsetY = 0.3                # optional: a frame-height fraction from centre
//!
//! [style]                      # any of a title's fields; the rest default
//! fontFamily = "Big Red"
//! fontSize = 0.1
//! fontWeight = 700
//! color = "#ff0000"
//! strokeColor = "#000000"
//! strokeWidth = 0.01
//! align = "center"
//! ```
//!
//! The font is installed when the preset is first used, not when it is
//! read: copied once into the app's own `fonts/` folder if it is not there
//! already, and registered on the project so the title painter finds it.
//! A preset used on two machines therefore renders the same on both, and
//! a project carries the font's path the way it carries any font added by
//! hand.

use std::path::{Path, PathBuf};

use concat_host::AppDirs;
use concat_project::model::{TextAlign, TextStyle};
use serde::Deserialize;

/// One look a title can be given.
pub struct TextPreset {
    /// Stable for ever; "default" is the plain title.
    pub id: String,
    pub name: String,
    /// The look, with `content` as the words the card places.
    pub style: TextStyle,
    /// Where the title sits, as a frame-height fraction from the centre,
    /// when the preset has an opinion - a lower third does.
    pub offset_y: Option<f64>,
    /// A font file the preset brings with it, resolved to a path.
    pub font: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresetFile {
    id: String,
    name: String,
    #[serde(default)]
    font: Option<String>,
    #[serde(default)]
    offset_y: Option<f64>,
    #[serde(default)]
    style: PresetStyle,
}

/// A title's fields, every one optional, laid over the default style.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PresetStyle {
    content: Option<String>,
    font_family: Option<String>,
    font_size: Option<f64>,
    font_weight: Option<f64>,
    italic: Option<bool>,
    color: Option<String>,
    align: Option<String>,
    opacity: Option<f64>,
    stroke_width: Option<f64>,
    stroke_color: Option<String>,
    shadow: Option<bool>,
    background: Option<String>,
    line_height: Option<f64>,
    tracking: Option<f64>,
}

impl PresetStyle {
    fn over(self, name: &str) -> TextStyle {
        let base = TextStyle::default();
        TextStyle {
            content: self.content.unwrap_or_else(|| name.to_owned()),
            font_family: self.font_family.unwrap_or_else(|| "Inter".to_owned()),
            font_size: self.font_size.unwrap_or(base.font_size).clamp(0.005, 1.0),
            font_weight: self
                .font_weight
                .unwrap_or(base.font_weight)
                .clamp(100.0, 900.0),
            italic: self.italic.unwrap_or(false),
            color: self.color.unwrap_or(base.color),
            align: match self.align.as_deref() {
                Some("left") => TextAlign::Left,
                Some("right") => TextAlign::Right,
                _ => TextAlign::Center,
            },
            opacity: self.opacity.unwrap_or(1.0).clamp(0.0, 1.0),
            stroke_width: self.stroke_width.unwrap_or(0.0).max(0.0),
            stroke_color: self.stroke_color.unwrap_or(base.stroke_color),
            shadow: self.shadow.unwrap_or(true),
            background: self.background.unwrap_or_default(),
            line_height: self.line_height.unwrap_or(base.line_height).max(0.5),
            tracking: self.tracking.unwrap_or(0.0),
        }
    }
}

/// Where the user's presets live.
pub fn dir(dirs: &AppDirs) -> PathBuf {
    dirs.config.join("text-presets")
}

/// The presets that ship with the app.
pub fn builtin() -> Vec<TextPreset> {
    let look = |id: &str, name: &str, style: TextStyle, offset_y: Option<f64>| TextPreset {
        id: id.to_owned(),
        name: name.to_owned(),
        style,
        offset_y,
        font: None,
    };
    let inter = |content: &str, weight: f64, size: f64| TextStyle {
        content: content.to_owned(),
        font_family: "Inter".to_owned(),
        font_weight: weight,
        font_size: size,
        ..TextStyle::default()
    };
    vec![
        look("default", "Title", inter("New title", 600.0, 0.09), None),
        look(
            "concat.headline",
            "Headline",
            TextStyle {
                stroke_width: 0.008,
                ..inter("Headline", 700.0, 0.12)
            },
            None,
        ),
        look(
            "concat.subtitle",
            "Subtitle",
            TextStyle {
                background: "#000000b3".to_owned(),
                shadow: false,
                ..inter("Subtitle", 500.0, 0.045)
            },
            Some(0.36),
        ),
        look(
            "concat.lower-third",
            "Lower third",
            TextStyle {
                color: "#10160a".to_owned(),
                background: "#c6f432".to_owned(),
                align: TextAlign::Left,
                shadow: false,
                ..inter("Name — Title", 600.0, 0.05)
            },
            Some(0.32),
        ),
        look(
            "concat.caption",
            "Caption",
            TextStyle {
                color: "#ffe14a".to_owned(),
                stroke_width: 0.006,
                ..inter("Caption", 600.0, 0.05)
            },
            Some(0.35),
        ),
        look(
            "concat.elegant",
            "Elegant",
            TextStyle {
                font_family: "Synonym".to_owned(),
                italic: true,
                tracking: 0.01,
                shadow: false,
                ..inter("Elegant", 400.0, 0.08)
            },
            None,
        ),
        look(
            "concat.neon",
            "Neon",
            TextStyle {
                color: "#c6f432".to_owned(),
                stroke_color: "#1c3b06".to_owned(),
                stroke_width: 0.006,
                ..inter("Neon", 700.0, 0.1)
            },
            None,
        ),
        look(
            "concat.outline",
            "Outline",
            TextStyle {
                stroke_width: 0.012,
                shadow: false,
                ..inter("Outline", 700.0, 0.11)
            },
            None,
        ),
        look(
            "concat.minimal",
            "Minimal",
            TextStyle {
                tracking: 0.03,
                shadow: false,
                ..inter("Minimal", 400.0, 0.06)
            },
            None,
        ),
    ]
}

/// The presets in the user's folder. A file that does not parse is
/// skipped: one broken preset should not empty the page.
pub fn user(dirs: &AppDirs) -> Vec<TextPreset> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir(dirs)) else {
        return found;
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter_map(|path| {
            if path.is_dir() {
                let inner = path.join("preset.toml");
                inner.is_file().then_some(inner)
            } else {
                (path.extension().and_then(|e| e.to_str()) == Some("toml")).then_some(path)
            }
        })
        .collect();
    paths.sort();
    for path in paths {
        if let Some(preset) = read(&path) {
            found.push(preset);
        }
    }
    found
}

fn read(path: &Path) -> Option<TextPreset> {
    let text = std::fs::read_to_string(path).ok()?;
    let file: PresetFile = toml::from_str(&text).ok()?;
    let id = file.id.trim().to_owned();
    if id.is_empty() {
        return None;
    }
    let font = file
        .font
        .filter(|name| !name.trim().is_empty())
        .map(|name| path.parent().unwrap_or(Path::new(".")).join(name.trim()));
    Some(TextPreset {
        name: if file.name.trim().is_empty() {
            id.clone()
        } else {
            file.name.trim().to_owned()
        },
        id,
        style: file.style.over(&file.name),
        offset_y: file.offset_y,
        font,
    })
}

/// Every preset the page offers: the built-in ones, then the user's. A
/// user preset with a built-in id replaces it, which is how a look can be
/// overridden without a second card for it.
pub fn all(dirs: &AppDirs) -> Vec<TextPreset> {
    let mut presets = builtin();
    for preset in user(dirs) {
        match presets.iter().position(|held| held.id == preset.id) {
            Some(index) => presets[index] = preset,
            None => presets.push(preset),
        }
    }
    presets
}

/// Puts the preset's font where the app keeps fonts, if it is not there
/// already, and says how to register it: the family the style names and
/// the installed file. None for a preset that brings no font, or whose
/// file cannot be read.
pub fn install_font(dirs: &AppDirs, preset: &TextPreset) -> Option<(String, String)> {
    let source = preset.font.as_ref().filter(|path| path.is_file())?;
    let name = source.file_name()?;
    let fonts = dirs.config.join("fonts");
    let installed = fonts.join(name);
    if !installed.is_file() {
        std::fs::create_dir_all(&fonts).ok()?;
        std::fs::copy(source, &installed).ok()?;
    }
    let family = preset.style.font_family.trim().trim_matches('"').to_owned();
    if family.is_empty() {
        return None;
    }
    Some((family, installed.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_ids_are_unique_and_the_plain_title_is_first() {
        let presets = builtin();
        assert_eq!(presets[0].id, "default");
        for (index, preset) in presets.iter().enumerate() {
            assert!(
                !presets[..index].iter().any(|held| held.id == preset.id),
                "{} twice",
                preset.id
            );
        }
    }

    #[test]
    fn a_partial_style_lays_over_the_default() {
        let file: PresetFile = toml::from_str(
            r##"
            id = "t.red"
            name = "Red"
            offsetY = 0.3
            [style]
            color = "#ff0000"
            align = "left"
            "##,
        )
        .expect("parses");
        let style = file.style.over(&file.name);
        assert_eq!(style.color, "#ff0000");
        assert_eq!(style.align, TextAlign::Left);
        assert_eq!(style.content, "Red");
        assert_eq!(style.font_weight, TextStyle::default().font_weight);
        assert_eq!(file.offset_y, Some(0.3));
    }
}

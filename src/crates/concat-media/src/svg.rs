// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! SVG, which FFmpeg cannot draw.
//!
//! FFmpeg reads an SVG only through librsvg, which no build of Concat
//! links; without it the file opened as a picture with no size, and the
//! import said "video stream has no usable size". The SVG is drawn once
//! with resvg - already in the build, through Slint - into a PNG in the
//! temporary folder, and everything that reads pictures reads that: the
//! probe, the monitor, the cards, proxies and export alike. The project
//! keeps the SVG's own path, so a changed file is drawn again.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The long side of the drawing, in pixels: a 4K frame's width, so a logo
/// laid over a UHD export is as sharp as the canvas. A vector has no size
/// of its own worth keeping - an icon says 24 px or 800 px for the same
/// shape.
const LONG_SIDE: f32 = 3840.0;

/// Whether `path` names an SVG, by its extension.
pub fn is_svg(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("svg") || extension.eq_ignore_ascii_case("svgz")
        })
}

/// What FFmpeg should open for `path`: the SVG's drawing, drawn now if it
/// is not drawn already, or `path` itself when it is not an SVG.
pub fn readable(path: &Path) -> Result<PathBuf> {
    if !is_svg(path) {
        return Ok(path.to_path_buf());
    }
    let fail = |detail: String| Error::Probe {
        path: path.to_path_buf(),
        detail,
    };
    let metadata = std::fs::metadata(path).map_err(|error| fail(error.to_string()))?;
    let drawn = drawing_of(path, &metadata);
    if drawn.is_file() {
        return Ok(drawn);
    }
    let bytes = std::fs::read(path).map_err(|error| fail(error.to_string()))?;
    let png = draw(&bytes).map_err(fail)?;
    if let Some(dir) = drawn.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Written beside and renamed over, so a reader on another thread never
    // opens half a PNG.
    let partial = drawn.with_extension(format!("{}.part", std::process::id()));
    std::fs::write(&partial, png).map_err(|error| fail(error.to_string()))?;
    std::fs::rename(&partial, &drawn).map_err(|error| fail(error.to_string()))?;
    Ok(drawn)
}

/// Where the drawing of `path`, as it is now, lives: named for its path,
/// size and modification time, so an edited SVG gets a new drawing.
fn drawing_of(path: &Path, metadata: &std::fs::Metadata) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata.modified().ok().hash(&mut hasher);
    LONG_SIDE.to_bits().hash(&mut hasher);
    std::env::temp_dir()
        .join("concat-svg")
        .join(format!("{:016x}.png", hasher.finish()))
}

/// The SVG in `bytes`, drawn with its long side at [`LONG_SIDE`], as a PNG.
fn draw(bytes: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_data(bytes, &options)
        .map_err(|error| format!("not an SVG Concat can read: {error}"))?;
    let size = tree.size();
    let scale = LONG_SIDE / size.width().max(size.height());
    let width = (size.width() * scale).round().max(1.0) as u32;
    let height = (size.height() * scale).round().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| format!("an SVG of {width}×{height} is too large to draw"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|error| format!("could not write the SVG's drawing: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELIX: &str = r##"<svg width="800px" height="400px" viewBox="0 0 1024 512" xmlns="http://www.w3.org/2000/svg"><rect x="8" y="8" width="1000" height="490" fill="#c0ff33"/></svg>"##;

    #[test]
    fn an_svg_is_drawn_at_the_long_side_and_probes_as_a_still() {
        let dir = std::env::temp_dir().join(format!("concat-svg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("helix.svg");
        std::fs::write(&svg, HELIX).unwrap();

        let drawn = readable(&svg).unwrap();
        assert_ne!(drawn, svg);
        assert_eq!(readable(&svg).unwrap(), drawn, "drawn once");

        let info = crate::probe(&svg).unwrap();
        assert_eq!(info.path, svg, "the project keeps the SVG");
        let video = info.video.expect("a picture");
        assert_eq!((video.width, video.height), (3840, 1920));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn not_an_svg_says_so() {
        let dir = std::env::temp_dir().join(format!("concat-svg-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("broken.svg");
        std::fs::write(&svg, "not xml at all").unwrap();
        let error = readable(&svg).unwrap_err().to_string();
        assert!(error.contains("not an SVG"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn other_files_are_read_as_they_are() {
        let path = Path::new("clip.mp4");
        assert_eq!(readable(path).unwrap(), path);
        assert!(is_svg(Path::new("logo.SVG")));
    }
}

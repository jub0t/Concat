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
use std::sync::atomic::{AtomicU64, Ordering};

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
    let png = draw(&bytes, path.parent()).map_err(fail)?;
    if let Some(dir) = drawn.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Written beside and renamed over, so a reader on another thread never
    // opens half a PNG. Beside under a name of this call's own: the
    // monitor, the prefetcher and an export's decoders all open the file,
    // and when its drawing is not there yet - a new SVG, an edited one, or
    // one the system's cleaner took out of the temporary folder - several
    // draw it at once. Under one name per process they wrote into each
    // other's file, so one renamed a PNG another was still writing, which
    // then stood as the drawing for good, and the others' renames found
    // nothing to move and failed the decoder: the SVG was missing from the
    // export.
    static DRAWING: AtomicU64 = AtomicU64::new(0);
    let partial = drawn.with_extension(format!(
        "{}-{}.part",
        std::process::id(),
        DRAWING.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&partial, png).map_err(|error| fail(error.to_string()))?;
    if let Err(error) = std::fs::rename(&partial, &drawn) {
        let _ = std::fs::remove_file(&partial);
        // Another drawing of the same file landed first; it is as good.
        if !drawn.is_file() {
            return Err(fail(error.to_string()));
        }
    }
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
/// `folder` is the one the file is in, where a picture it names by a
/// relative path - `<image href="photo.png">`, as a design tool writes a
/// placed bitmap - is found; without it such pictures were looked for
/// beside the app and left out of the drawing.
fn draw(bytes: &[u8], folder: Option<&Path>) -> std::result::Result<Vec<u8>, String> {
    let mut options = resvg::usvg::Options {
        resources_dir: folder.map(Path::to_path_buf),
        ..resvg::usvg::Options::default()
    };
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

    /// Everything that reads pictures asks for a drawing not made yet at
    /// once - the monitor, the prefetcher, an export's decoders - and every
    /// one of them gets it, whole.
    #[test]
    fn an_svg_asked_for_on_many_threads_at_once_is_drawn_for_all_of_them() {
        let dir = std::env::temp_dir().join(format!("concat-svg-race-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("race.svg");
        std::fs::write(&svg, HELIX).unwrap();

        let drawings: Vec<PathBuf> = std::thread::scope(|scope| {
            let asks: Vec<_> = (0..8).map(|_| scope.spawn(|| readable(&svg))).collect();
            asks.into_iter()
                .map(|ask| ask.join().expect("no panic").expect("drawn"))
                .collect()
        });
        assert!(drawings.iter().all(|drawn| *drawn == drawings[0]));
        let png = std::fs::read(&drawings[0]).unwrap();
        let pixmap = resvg::tiny_skia::Pixmap::decode_png(&png).expect("a whole PNG");
        assert_eq!((pixmap.width(), pixmap.height()), (3840, 1920));
        let leftovers = std::fs::read_dir(drawings[0].parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let stem = drawings[0].file_stem().unwrap().to_string_lossy().into_owned();
                name.starts_with(&stem)
            })
            .count();
        assert_eq!(leftovers, 0, "no half-written drawing is left behind");
        let _ = std::fs::remove_file(&drawings[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A picture the SVG places by a path relative to itself is drawn:
    /// it is looked for beside the SVG.
    #[test]
    fn a_picture_named_beside_the_svg_is_drawn() {
        let dir = std::env::temp_dir().join(format!("concat-svg-image-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut dot = resvg::tiny_skia::Pixmap::new(4, 4).unwrap();
        dot.fill(resvg::tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        std::fs::write(dir.join("dot.png"), dot.encode_png().unwrap()).unwrap();
        let svg = dir.join("placed.svg");
        std::fs::write(
            &svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100"><image x="0" y="0" width="100" height="100" xlink:href="dot.png"/></svg>"#,
        )
        .unwrap();

        let drawn = readable(&svg).unwrap();
        let pixmap = resvg::tiny_skia::Pixmap::decode_png(&std::fs::read(&drawn).unwrap())
            .expect("a whole PNG");
        let middle = pixmap.pixel(1920, 1920).expect("inside");
        assert_eq!(
            (middle.red(), middle.green(), middle.blue(), middle.alpha()),
            (255, 0, 0, 255),
            "the placed picture is in the drawing"
        );
        let _ = std::fs::remove_file(&drawn);
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

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Titles as pixels.
//!
//! A text clip is a style and some words; the compositor wants a picture. This
//! crate is the step between: it finds the face, shapes each line, turns the
//! glyph outlines into paths, and paints them - plate, shadow, outline, fill -
//! onto a canvas the size of the output frame, transparent everywhere the
//! words are not.
//!
//! Frame-sized on purpose. The compositor places a picture by fitting it into
//! the frame and then applying the clip's transform about its centre, so a
//! canvas that *is* the frame fits at exactly one, decodes without resampling,
//! and puts the block's centre where the clip's centre is. The clip's offset
//! and rotation then mean the same thing for a title as for footage.
//!
//! Sizes in the style are fractions of the frame's height, as the document
//! stores them, so a title looks the same at 720p and 4K. Everything here
//! converts to pixels once, at the top.

use std::fmt;

use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect,
    Stroke, Transform,
};

/// How a title's lines sit within their block.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Align {
    /// Lines share a left edge.
    Left,
    /// Lines are centred on each other.
    #[default]
    Center,
    /// Lines share a right edge.
    Right,
}

/// Everything about a title's look. Mirrors the document's text style field
/// for field so the host can copy it across; this crate does not depend on
/// the document.
#[derive(Clone, PartialEq, Debug)]
pub struct TitleStyle {
    /// The words, newlines included.
    pub content: String,
    /// CSS-style family name; quotes are tolerated and stripped.
    pub font_family: String,
    /// Em size as a fraction of frame height.
    pub font_size: f64,
    /// CSS-scale weight, 100..=900.
    pub font_weight: f64,
    /// Italic when true.
    pub italic: bool,
    /// Fill colour as `#rrggbb` or `#rrggbbaa`.
    pub color: String,
    /// Line alignment within the block.
    pub align: Align,
    /// Outline thickness as a fraction of frame height; zero for none.
    pub stroke_width: f64,
    /// Outline colour.
    pub stroke_color: String,
    /// A soft drop shadow behind the words.
    pub shadow: bool,
    /// A plate behind the block, `#rrggbb[aa]`; empty for none.
    pub background: String,
    /// Baseline pitch as a multiple of the em.
    pub line_height: f64,
    /// Extra advance after every glyph, as a fraction of frame height.
    pub tracking: f64,
}

/// The finished title.
#[derive(Clone, PartialEq, Debug)]
pub struct Rendered {
    /// The canvas, PNG-encoded, RGBA with alpha.
    pub png: Vec<u8>,
    /// The canvas width: the frame's.
    pub width: u32,
    /// The canvas height: the frame's.
    pub height: u32,
    /// The painted block's width in pixels, plate included - what an
    /// outline on a monitor should be drawn around.
    pub block_width: u32,
    /// The painted block's height, on the same terms.
    pub block_height: u32,
}

/// What can go wrong. Fonts fall back rather than fail, so this is short.
#[derive(Debug)]
pub enum Error {
    /// The frame is zero-sized or too large for a pixmap.
    Canvas(u32, u32),
    /// No face at all could be found, not even a system fallback.
    NoFont,
    /// A face was found but its data could not be read as a font.
    BadFont,
    /// PNG encoding failed.
    Encode(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Canvas(w, h) => write!(f, "cannot make a {w}×{h} canvas"),
            Error::NoFont => write!(f, "no font found, not even a system fallback"),
            Error::BadFont => write!(f, "the chosen font file could not be parsed"),
            Error::Encode(why) => write!(f, "PNG encoding failed: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// The faces available to titles: the system's, plus any files a project
/// carries. Built once and kept; loading the system's fonts is the slow part.
pub struct Fonts {
    db: fontdb::Database,
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl Fonts {
    /// The system's fonts.
    pub fn new() -> Fonts {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Fonts { db }
    }

    /// Adds one font file. A file that does not parse is skipped; a title
    /// that names its family falls back to a system face.
    pub fn add_file(&mut self, path: &std::path::Path) -> bool {
        self.db.load_font_file(path).is_ok()
    }

    /// The best face for a style: the named family at the nearest weight and
    /// slant, then any sans-serif, then anything at all.
    fn pick(&self, style: &TitleStyle) -> Result<Vec<u8>, Error> {
        let family = style
            .font_family
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        let weight = fontdb::Weight(style.font_weight.clamp(100.0, 900.0).round() as u16);
        let slant = if style.italic {
            fontdb::Style::Italic
        } else {
            fontdb::Style::Normal
        };
        let mut families: Vec<fontdb::Family<'_>> = Vec::new();
        if !family.is_empty() {
            families.push(fontdb::Family::Name(family));
        }
        families.push(fontdb::Family::SansSerif);
        let query = fontdb::Query {
            families: &families,
            weight,
            stretch: fontdb::Stretch::Normal,
            style: slant,
        };
        let id = self
            .db
            .query(&query)
            .or_else(|| self.db.faces().next().map(|face| face.id))
            .ok_or(Error::NoFont)?;
        // Copied out: the shaper and the outliner both want a slice that
        // outlives the database borrow, and a face is a few hundred KB.
        self.db
            .with_face_data(id, |data, index| {
                // Multi-face collections: keep only the face that answered.
                // rustybuzz takes the index, so the whole blob travels.
                (data.to_vec(), index)
            })
            .map(|(data, index)| {
                // Encode the index in front so the caller can hand both on.
                let mut out = index.to_le_bytes().to_vec();
                out.extend(data);
                out
            })
            .ok_or(Error::BadFont)
    }
}

/// One shaped line: its outline path in pixels, pen at the origin, and its
/// advance width.
struct Line {
    path: Option<Path>,
    width: f32,
}

/// A colour from `#rrggbb` or `#rrggbbaa`; anything else is `None`.
fn colour(hex: &str) -> Option<Color> {
    let hex = hex.trim().strip_prefix('#')?;
    let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    match hex.len() {
        6 => Color::from_rgba8(byte(0)?, byte(2)?, byte(4)?, 255).into(),
        8 => Color::from_rgba8(byte(0)?, byte(2)?, byte(4)?, byte(6)?).into(),
        _ => None,
    }
}

/// Collects a glyph's outline, scaled and placed, into a path under
/// construction. The font's y goes up; the canvas's goes down.
struct Outliner<'a> {
    builder: &'a mut PathBuilder,
    scale: f32,
    x: f32,
    y: f32,
}

impl ttf_parser::OutlineBuilder for Outliner<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder
            .move_to(self.x + x * self.scale, self.y - y * self.scale);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.builder
            .line_to(self.x + x * self.scale, self.y - y * self.scale);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.builder.quad_to(
            self.x + x1 * self.scale,
            self.y - y1 * self.scale,
            self.x + x * self.scale,
            self.y - y * self.scale,
        );
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.builder.cubic_to(
            self.x + x1 * self.scale,
            self.y - y1 * self.scale,
            self.x + x2 * self.scale,
            self.y - y2 * self.scale,
            self.x + x * self.scale,
            self.y - y * self.scale,
        );
    }
    fn close(&mut self) {
        self.builder.close();
    }
}

/// Shapes one line and outlines it, pen starting at (0, 0) on the baseline.
fn shape_line(face: &rustybuzz::Face<'_>, text: &str, em: f32, tracking: f32) -> Line {
    if text.is_empty() {
        return Line {
            path: None,
            width: 0.0,
        };
    }
    let scale = em / face.units_per_em() as f32;
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    let shaped = rustybuzz::shape(face, &[], buffer);
    let mut builder = PathBuilder::new();
    let mut pen = 0.0_f32;
    for (info, position) in shaped
        .glyph_infos()
        .iter()
        .zip(shaped.glyph_positions().iter())
    {
        let glyph = ttf_parser::GlyphId(info.glyph_id as u16);
        let mut outliner = Outliner {
            builder: &mut builder,
            scale,
            x: pen + position.x_offset as f32 * scale,
            y: -(position.y_offset as f32 * scale),
        };
        face.outline_glyph(glyph, &mut outliner);
        pen += position.x_advance as f32 * scale + tracking;
    }
    // The tracking after the last glyph is air nobody sees.
    let width = (pen - tracking).max(0.0);
    Line {
        path: builder.finish(),
        width,
    }
}

/// A separable box blur over premultiplied RGBA, run twice for a soft
/// falloff. Radius in pixels; zero leaves the picture alone.
fn blur(pixmap: &mut Pixmap, radius: usize) {
    if radius == 0 {
        return;
    }
    let (width, height) = (pixmap.width() as usize, pixmap.height() as usize);
    let data = pixmap.data_mut();
    let mut scratch = vec![0u8; data.len()];
    for _ in 0..2 {
        // Horizontal, into scratch.
        for y in 0..height {
            let row = y * width * 4;
            for channel in 0..4 {
                let mut sum: u32 = 0;
                let window = (2 * radius + 1) as u32;
                let at = |x: isize| -> u32 {
                    let x = x.clamp(0, width as isize - 1) as usize;
                    u32::from(data[row + x * 4 + channel])
                };
                for x in -(radius as isize)..=(radius as isize) {
                    sum += at(x);
                }
                for x in 0..width {
                    scratch[row + x * 4 + channel] = (sum / window) as u8;
                    sum += at(x as isize + radius as isize + 1);
                    sum -= at(x as isize - radius as isize);
                }
            }
        }
        // Vertical, back into data.
        for x in 0..width {
            for channel in 0..4 {
                let mut sum: u32 = 0;
                let window = (2 * radius + 1) as u32;
                let at = |y: isize| -> u32 {
                    let y = y.clamp(0, height as isize - 1) as usize;
                    u32::from(scratch[(y * width + x) * 4 + channel])
                };
                for y in -(radius as isize)..=(radius as isize) {
                    sum += at(y);
                }
                for y in 0..height {
                    data[(y * width + x) * 4 + channel] = (sum / window) as u8;
                    sum += at(y as isize + radius as isize + 1);
                    sum -= at(y as isize - radius as isize);
                }
            }
        }
    }
}

/// Paints `style` onto a `width` × `height` transparent canvas, the block
/// centred, and returns it PNG-encoded with the block's size.
pub fn render(
    fonts: &Fonts,
    style: &TitleStyle,
    width: u32,
    height: u32,
) -> Result<Rendered, Error> {
    let mut canvas = Pixmap::new(width, height).ok_or(Error::Canvas(width, height))?;
    let frame_h = height as f32;
    let em = (style.font_size.clamp(0.005, 1.0) as f32) * frame_h;
    let tracking = style.tracking as f32 * frame_h;
    let pitch = em * (style.line_height.max(0.5) as f32);

    let blob = fonts.pick(style)?;
    let (index_bytes, data) = blob.split_at(4);
    let index = u32::from_le_bytes([
        index_bytes[0],
        index_bytes[1],
        index_bytes[2],
        index_bytes[3],
    ]);
    let face = rustybuzz::Face::from_slice(data, index).ok_or(Error::BadFont)?;
    let upem = face.units_per_em() as f32;
    let ascent = face.ascender() as f32 / upem * em;
    let descent = -(face.descender() as f32) / upem * em;

    // Shape every line with the pen at the origin; placement comes after,
    // once the block's width is known.
    let lines: Vec<Line> = style
        .content
        .lines()
        .map(|line| shape_line(&face, line, em, tracking))
        .collect();
    let rows = lines.len().max(1);
    let block_w = lines.iter().map(|line| line.width).fold(0.0, f32::max);
    let block_h = (rows as f32 - 1.0) * pitch + ascent + descent;
    if block_w <= 0.0 || lines.is_empty() {
        // Nothing to paint: an empty, valid canvas.
        let png = canvas
            .encode_png()
            .map_err(|error| Error::Encode(error.to_string()))?;
        return Ok(Rendered {
            png,
            width,
            height,
            block_width: 0,
            block_height: 0,
        });
    }

    // The plate's padding is part of the block: it is what a monitor should
    // outline, and what the title's neighbours should keep clear of.
    let plate = colour(&style.background);
    let (pad_x, pad_y) = if plate.is_some() {
        (em * 0.35, em * 0.2)
    } else {
        (0.0, 0.0)
    };
    let outer_w = block_w + 2.0 * pad_x;
    let outer_h = block_h + 2.0 * pad_y;
    let left = (width as f32 - outer_w) / 2.0 + pad_x;
    let top = (frame_h - outer_h) / 2.0 + pad_y;

    // One path for all the words, placed. Each line is aligned within the
    // block's width and sits on its own baseline.
    let mut words = PathBuilder::new();
    for (row, line) in lines.iter().enumerate() {
        let Some(path) = &line.path else { continue };
        let indent = match style.align {
            Align::Left => 0.0,
            Align::Center => (block_w - line.width) / 2.0,
            Align::Right => block_w - line.width,
        };
        let baseline = top + ascent + row as f32 * pitch;
        let placed = path
            .clone()
            .transform(Transform::from_translate(left + indent, baseline))
            .expect("a translated glyph path stays finite");
        words.push_path(&placed);
    }
    let Some(words) = words.finish() else {
        let png = canvas
            .encode_png()
            .map_err(|error| Error::Encode(error.to_string()))?;
        return Ok(Rendered {
            png,
            width,
            height,
            block_width: outer_w.round() as u32,
            block_height: outer_h.round() as u32,
        });
    };

    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };

    // The plate, first and under everything.
    if let Some(fill) = plate
        && let Some(rect) = Rect::from_xywh(left - pad_x, top - pad_y, outer_w, outer_h)
    {
        paint.set_color(fill);
        let radius = em * 0.15;
        let mut plate_path = PathBuilder::new();
        push_rounded_rect(&mut plate_path, rect, radius);
        if let Some(plate_path) = plate_path.finish() {
            canvas.fill_path(
                &plate_path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    // The shadow: the words again, offset down and right, black, blurred on
    // a layer of their own and laid under the real words.
    if style.shadow
        && let Some(mut layer) = Pixmap::new(width, height)
    {
        let mut shade = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        shade.set_color(Color::from_rgba8(0, 0, 0, 150));
        let offset = Transform::from_translate(em * 0.05, em * 0.07);
        layer.fill_path(&words, &shade, FillRule::Winding, offset, None);
        blur(&mut layer, (em * 0.04).round().max(1.0) as usize);
        canvas.draw_pixmap(
            0,
            0,
            layer.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }

    // The outline, under the fill so only its outer half shows - which is
    // why it is drawn at twice the asked-for width.
    let stroke_w = style.stroke_width.max(0.0) as f32 * frame_h;
    if stroke_w > 0.0
        && let Some(edge) = colour(&style.stroke_color)
    {
        paint.set_color(edge);
        let stroke = Stroke {
            width: stroke_w * 2.0,
            line_join: LineJoin::Round,
            line_cap: LineCap::Round,
            ..Stroke::default()
        };
        canvas.stroke_path(&words, &paint, &stroke, Transform::identity(), None);
    }

    // The words.
    paint.set_color(colour(&style.color).unwrap_or(Color::WHITE));
    canvas.fill_path(
        &words,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );

    let png = canvas
        .encode_png()
        .map_err(|error| Error::Encode(error.to_string()))?;
    Ok(Rendered {
        png,
        width,
        height,
        block_width: outer_w.round() as u32,
        block_height: outer_h.round() as u32,
    })
}

/// A rectangle with rounded corners, radius clamped to half the short side.
fn push_rounded_rect(builder: &mut PathBuilder, rect: Rect, radius: f32) {
    let r = radius
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0)
        .max(0.0);
    let (l, t, rgt, b) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    // Quadratic corners: close enough to circular at these sizes, and half
    // the segments of a cubic approximation.
    builder.move_to(l + r, t);
    builder.line_to(rgt - r, t);
    builder.quad_to(rgt, t, rgt, t + r);
    builder.line_to(rgt, b - r);
    builder.quad_to(rgt, b, rgt - r, b);
    builder.line_to(l + r, b);
    builder.quad_to(l, b, l, b - r);
    builder.line_to(l, t + r);
    builder.quad_to(l, t, l + r, t);
    builder.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(content: &str) -> TitleStyle {
        TitleStyle {
            content: content.to_owned(),
            font_family: "\"No Such Family\"".to_owned(),
            font_size: 0.09,
            font_weight: 700.0,
            italic: false,
            color: "#ffffff".to_owned(),
            align: Align::Center,
            stroke_width: 0.0,
            stroke_color: "#000000".to_owned(),
            shadow: true,
            background: String::new(),
            line_height: 1.2,
            tracking: 0.0,
        }
    }

    fn opaque_pixels(png: &[u8]) -> usize {
        let pixmap = Pixmap::decode_png(png).expect("our own PNG decodes");
        pixmap.pixels().iter().filter(|p| p.alpha() > 0).count()
    }

    #[test]
    fn colours_parse_both_lengths() {
        assert_eq!(colour("#ff0000"), Some(Color::from_rgba8(255, 0, 0, 255)));
        assert_eq!(colour("#00ff0080"), Some(Color::from_rgba8(0, 255, 0, 128)));
        assert_eq!(colour(""), None);
        assert_eq!(colour("red"), None);
    }

    /// A missing family falls back to a system face and still paints words.
    #[test]
    fn a_title_paints_something_with_a_fallback_face() {
        let fonts = Fonts::new();
        let out = render(&fonts, &style("Hello"), 640, 360).expect("renders");
        assert_eq!((out.width, out.height), (640, 360));
        assert!(out.block_width > 0 && out.block_height > 0);
        assert!(out.block_width < 640);
        assert!(opaque_pixels(&out.png) > 100);
    }

    /// The canvas is the frame; the block is not.
    #[test]
    fn empty_content_is_an_empty_canvas() {
        let fonts = Fonts::new();
        let out = render(&fonts, &style(""), 320, 180).expect("renders");
        assert_eq!((out.block_width, out.block_height), (0, 0));
        assert_eq!(opaque_pixels(&out.png), 0);
    }

    /// More lines, taller block; a plate grows it further.
    #[test]
    fn lines_and_plates_grow_the_block() {
        let fonts = Fonts::new();
        let one = render(&fonts, &style("One"), 640, 360).expect("renders");
        let two = render(&fonts, &style("One\nTwo"), 640, 360).expect("renders");
        assert!(two.block_height > one.block_height);
        let mut plated = style("One");
        plated.background = "#000000cc".to_owned();
        let plated = render(&fonts, &plated, 640, 360).expect("renders");
        assert!(plated.block_width > one.block_width);
        assert!(plated.block_height > one.block_height);
    }
}

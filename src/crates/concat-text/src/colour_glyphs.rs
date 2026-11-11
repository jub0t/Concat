// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Glyphs that bring their own colours: emoji, mostly.
//!
//! A font can draw a glyph as a picture rather than an outline - a bitmap
//! in a strike (`sbix` on Apple's faces, `CBDT` on Android's and Linux's),
//! or layers of shapes in colours of their own (`COLR`, on Windows and in
//! newer emoji faces, from plain solid layers to gradients, clips,
//! transforms and blends). Whether a glyph is one of these is the face's to
//! say, glyph by glyph; nothing here looks at what a character means.
//!
//! Such a glyph is drawn in its own colours and nothing of the title's look:
//! no fill, outline or shadow, laid over everything else. Where its colours
//! cannot be drawn - a picture that will not decode, a layer that names a
//! colour the palette lacks - it falls back to its outline, painted like a
//! letter, so nothing that should show is left out.
//!
//! Decoding a picture is the dear part, and a title is painted again on
//! every change: so each picture is made once, at the size it is drawn,
//! and kept in [`Pictures`] for every title and thread after.

mod paint_tree;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tiny_skia::{
    Color, FillRule, FilterQuality, IntSize, Paint, Path, Pixmap, PixmapPaint, Transform,
};
use ttf_parser::{GlyphId, RasterGlyphImage, RasterImageFormat, RgbaColor};

use paint_tree::Layering;

/// How a face draws a character, where it can.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Rendition {
    /// In the glyph's own colours.
    Colour,
    /// As an outline, in the title's colours - or it is a space.
    Outline,
}

/// How `face` draws `ch`, or `None` where it cannot: no glyph for it, or a
/// glyph with neither colours this crate can draw nor an outline.
pub(crate) fn rendition(face: &ttf_parser::Face<'_>, ch: char) -> Option<Rendition> {
    let glyph = face.glyph_index(ch)?;
    if paints_itself(face, glyph) {
        Some(Rendition::Colour)
    } else if ch.is_whitespace() || face.glyph_bounding_box(glyph).is_some() {
        Some(Rendition::Outline)
    } else {
        None
    }
}

/// Whether a glyph has colours of its own that can be drawn: a picture in
/// a format this crate decodes - checked, not decoded - or colour layers
/// that walk cleanly and put something somewhere.
fn paints_itself(face: &ttf_parser::Face<'_>, glyph: GlyphId) -> bool {
    face.glyph_raster_image(glyph, u16::MAX)
        .is_some_and(|image| decodable(&image))
        || (face.is_color_glyph(glyph) && paint_tree::shows_anything(face, glyph))
}

/// A strike picture in colour: PNG, or premultiplied BGRA. Monochrome and
/// grey strikes are not colour, and their glyph is drawn as its outline.
fn decodable(image: &RasterGlyphImage<'_>) -> bool {
    matches!(
        image.format,
        RasterImageFormat::PNG | RasterImageFormat::BitmapPremulBgra32
    )
}

/// One glyph drawn in its own colours, in its line's pen space: pen at the
/// origin on the baseline, y down.
pub(crate) enum Mark {
    /// A picture, stretched over the box from `(x, y)` that is `width` by
    /// `height`.
    Bitmap {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        picture: Arc<Pixmap>,
    },
    /// Solid layers, bottom first, each an outline and its colour.
    Layers(Vec<(Path, Color)>),
}

/// The text colour a font's layers ask for, from a look's fill.
pub(crate) fn font_colour(fill: Color) -> RgbaColor {
    let fill = fill.to_color_u8();
    RgbaColor::new(fill.red(), fill.green(), fill.blue(), fill.alpha())
}

/// Which face a kept picture came from: where its bytes are, which the
/// [`crate::Fonts`] that keeps the pictures holds open for as long as it
/// lives, and the face's place in its collection.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct FaceKey {
    bytes: usize,
    index: u32,
}

impl FaceKey {
    pub(crate) fn of(data: &[u8], index: u32) -> FaceKey {
        FaceKey {
            bytes: data.as_ptr() as usize,
            index,
        }
    }
}

/// A glyph on its way onto a line: its face, where it sits, how big, and
/// the colour its layers take for the text's own.
pub(crate) struct Glyph<'f, 'a> {
    pub(crate) face: &'f ttf_parser::Face<'a>,
    pub(crate) key: FaceKey,
    pub(crate) id: GlyphId,
    /// The pen on the baseline, in the line's pixels.
    pub(crate) x: f32,
    pub(crate) y: f32,
    /// The em, in pixels.
    pub(crate) em: f32,
    pub(crate) foreground: RgbaColor,
}

/// How many bytes of pictures are kept at most: a few hundred emoji at a
/// caption's size, a few dozen at a headline's.
const KEPT_BYTES: usize = 32 << 20;

/// What a kept entry costs beyond its pixels, so a cache of failures and
/// tiny pictures is bounded too.
const ENTRY_BYTES: usize = 64;

/// Pictures of colour glyphs, made once and kept: shared by every paint and
/// every thread that paints with the same faces, and bounded by bytes, the
/// longest unused going first.
pub(crate) struct Pictures {
    store: Mutex<Store>,
    limit: usize,
    /// How many strike pictures have been decoded, and how many layered
    /// glyphs painted into pictures: what a test counts.
    decoded: AtomicUsize,
    painted: AtomicUsize,
}

impl Default for Pictures {
    fn default() -> Self {
        Pictures::holding(KEPT_BYTES)
    }
}

/// What a picture is kept under.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Key {
    /// A strike's picture at the strike's own size.
    Strike {
        face: FaceKey,
        glyph: u16,
        ppem: u16,
    },
    /// That picture shrunk to the whole pixels it is drawn at.
    Shrunk {
        face: FaceKey,
        glyph: u16,
        ppem: u16,
        width: u32,
        height: u32,
    },
    /// Layers with more than solid fills, painted at an em in 64ths of a
    /// pixel, in the text colour given.
    Painted {
        face: FaceKey,
        glyph: u16,
        em: u32,
        foreground: [u8; 4],
    },
}

/// A made picture and where its top-left corner sits from the pen, in its
/// own pixels.
#[derive(Clone)]
struct Picture {
    pixels: Arc<Pixmap>,
    left: f32,
    top: f32,
}

impl Picture {
    fn at_pen(pixels: Pixmap) -> Picture {
        Picture {
            pixels: Arc::new(pixels),
            left: 0.0,
            top: 0.0,
        }
    }
}

/// One kept entry: the picture, or `None` for a glyph whose picture could
/// not be made, so the failure is not tried again on every paint.
struct Kept {
    picture: Option<Picture>,
    used: u64,
    bytes: usize,
}

#[derive(Default)]
struct Store {
    kept: HashMap<Key, Kept>,
    bytes: usize,
    clock: u64,
}

impl Pictures {
    /// Pictures kept up to `limit` bytes.
    fn holding(limit: usize) -> Pictures {
        Pictures {
            store: Mutex::default(),
            limit,
            decoded: AtomicUsize::new(0),
            painted: AtomicUsize::new(0),
        }
    }

    /// `glyph` in its own colours, or `None` for one to outline: most
    /// glyphs, and a colour glyph whose colours cannot be drawn. A strike
    /// picture is tried first, then colour layers.
    pub(crate) fn mark(&self, glyph: &Glyph<'_, '_>) -> Option<Mark> {
        if let Some(mark) = self.bitmap(glyph) {
            return Some(mark);
        }
        if glyph.face.is_color_glyph(glyph.id) {
            return self.layered(glyph);
        }
        None
    }

    /// A strike picture, boxed to the em.
    ///
    /// The strike asked for is the smallest at or above the em - less to
    /// decode than the largest, and a smaller step down - or the largest
    /// where none is that big. A strike in the font's pixels per em is
    /// scaled by em over those, so its pictures are an em whichever strike
    /// is used. The box starts at the pen moved by the picture's origin
    /// offsets, which a font gives from the pen to the picture's bottom-left
    /// corner, y up.
    fn bitmap(&self, glyph: &Glyph<'_, '_>) -> Option<Mark> {
        let image = strike_for(glyph.face, glyph.id, glyph.em)?;
        let k = glyph.em / f32::from(image.pixels_per_em.max(1));
        let (width, height) = (f32::from(image.width) * k, f32::from(image.height) * k);
        let x = glyph.x + f32::from(image.x) * k;
        let y = glyph.y - f32::from(image.y) * k - height;

        let ppem = image.pixels_per_em;
        let whole = Key::Strike {
            face: glyph.key,
            glyph: glyph.id.0,
            ppem,
        };
        let full = || {
            self.cached(whole, || {
                self.decoded.fetch_add(1, Ordering::Relaxed);
                decode(&image).map(Picture::at_pen)
            })
        };
        // Shrunk once, by averaging every strike pixel under each drawn
        // one: a big strike resampled at paint time from four neighbours
        // came out jagged at a caption's size.
        let (fit_w, fit_h) = (
            (width.round() as u32).max(1),
            (height.round() as u32).max(1),
        );
        let picture = if fit_w < u32::from(image.width) || fit_h < u32::from(image.height) {
            let fitted = Key::Shrunk {
                face: glyph.key,
                glyph: glyph.id.0,
                ppem,
                width: fit_w,
                height: fit_h,
            };
            self.cached(fitted, || {
                let full = full()?;
                Some(shrink(&full.pixels, fit_w, fit_h).map_or(full, Picture::at_pen))
            })?
        } else {
            full()?
        };
        Some(Mark::Bitmap {
            x,
            y,
            width,
            height,
            picture: picture.pixels,
        })
    }

    /// Colour layers. Solid layers alone are outlined straight onto the
    /// line, as crisp as the letters beside them; layers with gradients,
    /// clips, transforms or blends are painted once into a picture at the
    /// em and kept.
    fn layered(&self, glyph: &Glyph<'_, '_>) -> Option<Mark> {
        let em64 = (glyph.em * 64.0).round().max(1.0) as u32;
        let foreground = glyph.foreground;
        let key = Key::Painted {
            face: glyph.key,
            glyph: glyph.id.0,
            em: em64,
            foreground: [
                foreground.red,
                foreground.green,
                foreground.blue,
                foreground.alpha,
            ],
        };
        // Painted before: the layers need not be walked again.
        let painted =
            |picture: Option<Picture>| picture.map(|picture| placed(&picture, glyph, em64));
        if let Some(found) = self.lookup(key) {
            return painted(found);
        }
        let scale = glyph.em / glyph.face.units_per_em() as f32;
        match paint_tree::layers(glyph.face, glyph.id, (glyph.x, glyph.y), scale, foreground) {
            Layering::Solid(layers) if !layers.is_empty() => Some(Mark::Layers(layers)),
            Layering::Solid(_) | Layering::Failed => None,
            Layering::Rich => painted(self.cached(key, || {
                self.painted.fetch_add(1, Ordering::Relaxed);
                paint_tree::paint(glyph.face, glyph.id, em64 as f32 / 64.0, foreground)
            })),
        }
    }

    /// The picture kept under `key`, or `make`'s, kept for next time. The
    /// store is not held while a picture is made, so one thread decoding
    /// does not hold up another painting.
    fn cached(&self, key: Key, make: impl FnOnce() -> Option<Picture>) -> Option<Picture> {
        if let Some(found) = self.lookup(key) {
            return found;
        }
        let made = make();
        self.keep(key, made.clone());
        made
    }

    /// What is kept under `key`: `Some(None)` for a picture known not to
    /// be makeable, `None` for nothing kept.
    fn lookup(&self, key: Key) -> Option<Option<Picture>> {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        store.clock += 1;
        let now = store.clock;
        let kept = store.kept.get_mut(&key)?;
        kept.used = now;
        Some(kept.picture.clone())
    }

    /// Keeps `picture` under `key`, letting the longest unused go while
    /// the store is over its bytes. A picture bigger than the whole store
    /// is drawn and not kept.
    fn keep(&self, key: Key, picture: Option<Picture>) {
        let bytes = ENTRY_BYTES + picture.as_ref().map_or(0, |p| p.pixels.data().len());
        if bytes > self.limit {
            return;
        }
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        store.clock += 1;
        let used = store.clock;
        if let Some(old) = store.kept.insert(
            key,
            Kept {
                picture,
                used,
                bytes,
            },
        ) {
            store.bytes -= old.bytes;
        }
        store.bytes += bytes;
        if store.bytes > self.limit {
            // Down to three quarters, so the next few pictures do not
            // each pay for a sweep.
            let mut ages: Vec<(u64, Key)> = store
                .kept
                .iter()
                .map(|(key, kept)| (kept.used, *key))
                .collect();
            ages.sort_unstable_by_key(|&(used, _)| used);
            for (_, old) in ages {
                if store.bytes <= self.limit / 4 * 3 {
                    break;
                }
                if let Some(gone) = store.kept.remove(&old) {
                    store.bytes -= gone.bytes;
                }
            }
        }
    }

    /// How many strike pictures have been decoded.
    #[cfg(test)]
    fn decodes(&self) -> usize {
        self.decoded.load(Ordering::Relaxed)
    }

    /// How many layered glyphs have been painted into pictures.
    #[cfg(test)]
    fn paints(&self) -> usize {
        self.painted.load(Ordering::Relaxed)
    }

    /// The bytes kept.
    #[cfg(test)]
    fn bytes(&self) -> usize {
        self.store.lock().unwrap_or_else(|e| e.into_inner()).bytes
    }
}

/// A picture painted at an em of `em64` 64ths of a pixel, boxed for the
/// glyph's own em and placed at its pen.
fn placed(picture: &Picture, glyph: &Glyph<'_, '_>, em64: u32) -> Mark {
    let k = glyph.em / (em64 as f32 / 64.0);
    Mark::Bitmap {
        x: glyph.x + picture.left * k,
        y: glyph.y + picture.top * k,
        width: picture.pixels.width() as f32 * k,
        height: picture.pixels.height() as f32 * k,
        picture: picture.pixels.clone(),
    }
}

/// The strike picture for `glyph` at `em` pixels, if the face has one in
/// colour: from the smallest strike at or above the em, or from the
/// largest where the glyph is missing from that one.
fn strike_for<'f>(
    face: &'f ttf_parser::Face<'_>,
    glyph: GlyphId,
    em: f32,
) -> Option<RasterGlyphImage<'f>> {
    let wanted = em.ceil().clamp(1.0, f32::from(u16::MAX)) as u16;
    match face.glyph_raster_image(glyph, wanted) {
        Some(image) => decodable(&image).then_some(image),
        None if wanted < u16::MAX => face
            .glyph_raster_image(glyph, u16::MAX)
            .filter(|image| decodable(image)),
        None => None,
    }
}

/// A strike picture as premultiplied RGBA: a PNG decoded, or BGRA - already
/// premultiplied - with red and blue swapped. `None` for a PNG that will
/// not decode, or BGRA whose length is not its stated size.
fn decode(image: &RasterGlyphImage<'_>) -> Option<Pixmap> {
    match image.format {
        RasterImageFormat::PNG => Pixmap::decode_png(image.data).ok(),
        RasterImageFormat::BitmapPremulBgra32 => {
            let size = IntSize::from_wh(u32::from(image.width), u32::from(image.height))?;
            if image.data.len() != size.width() as usize * size.height() as usize * 4 {
                return None;
            }
            let mut rgba = image.data.to_vec();
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
                // A colour above its alpha is not premultiplied; held to
                // it, a stray byte cannot overflow when composited.
                let alpha = pixel[3];
                for channel in &mut pixel[..3] {
                    *channel = (*channel).min(alpha);
                }
            }
            Pixmap::from_vec(rgba, size)
        }
        _ => None,
    }
}

/// `source` shrunk to `width` by `height`, each new pixel the average of
/// the source pixels under it, weighted by how much of each it covers.
/// `None` where that would not shrink it.
fn shrink(source: &Pixmap, width: u32, height: u32) -> Option<Pixmap> {
    let (from_w, from_h) = (source.width(), source.height());
    if width == 0
        || height == 0
        || width > from_w
        || height > from_h
        || (width, height) == (from_w, from_h)
    {
        return None;
    }
    let across = footprints(from_w, width);
    let down = footprints(from_h, height);
    let (width_px, from_w_px) = (width as usize, from_w as usize);
    let pixels = source.data();
    // Each row narrowed first, kept unrounded, then each column shortened.
    let mut narrowed = vec![0.0_f32; width_px * from_h as usize * 4];
    for (row, out) in pixels
        .chunks_exact(from_w_px * 4)
        .zip(narrowed.chunks_exact_mut(width_px * 4))
    {
        for (taps, sum) in across.iter().zip(out.chunks_exact_mut(4)) {
            for &(at, weight) in taps {
                for (total, &value) in sum.iter_mut().zip(&row[at * 4..at * 4 + 4]) {
                    *total += f32::from(value) * weight;
                }
            }
        }
    }
    let mut out = Pixmap::new(width, height)?;
    for (taps, row) in down
        .iter()
        .zip(out.data_mut().chunks_exact_mut(width_px * 4))
    {
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            let mut sum = [0.0_f32; 4];
            for &(at, weight) in taps {
                let from = &narrowed[(at * width_px + x) * 4..(at * width_px + x) * 4 + 4];
                for (total, value) in sum.iter_mut().zip(from) {
                    *total += value * weight;
                }
            }
            let byte = |value: f32| value.round().clamp(0.0, 255.0) as u8;
            let alpha = byte(sum[3]);
            for channel in 0..3 {
                pixel[channel] = byte(sum[channel]).min(alpha);
            }
            pixel[3] = alpha;
        }
    }
    Some(out)
}

/// For each of `to` pixels along an axis `from` pixels long, the source
/// pixels it covers and the share of each, the shares adding up to one.
fn footprints(from: u32, to: u32) -> Vec<Vec<(usize, f32)>> {
    let step = from as f32 / to as f32;
    (0..to)
        .map(|index| {
            let (start, end) = (index as f32 * step, (index + 1) as f32 * step);
            let mut taps = Vec::new();
            let mut at = start.floor() as usize;
            while (at as f32) < end && at < from as usize {
                let cover = end.min(at as f32 + 1.0) - start.max(at as f32);
                if cover > 0.0 {
                    taps.push((at, cover / step));
                }
                at += 1;
            }
            taps
        })
        .collect()
}

/// Lays a line's colour glyphs onto `canvas` with the line's pen at
/// `origin`, in order, source over. A picture at its own size is copied
/// pixel for pixel on whole pixels, rather than resampled half a pixel off
/// and softened; any other is scaled with bilinear filtering.
pub(crate) fn lay(canvas: &mut Pixmap, marks: &[Mark], origin: (f32, f32)) {
    let (origin_x, origin_y) = origin;
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    let at_pen = Transform::from_translate(origin_x, origin_y);
    for mark in marks {
        match mark {
            Mark::Bitmap {
                x,
                y,
                width,
                height,
                picture,
            } => {
                let pixels: &Pixmap = picture;
                let (left, top) = (origin_x + x, origin_y + y);
                let (own_w, own_h) = (pixels.width() as f32, pixels.height() as f32);
                if (width - own_w).abs() < 0.5 && (height - own_h).abs() < 0.5 {
                    canvas.draw_pixmap(
                        left.round() as i32,
                        top.round() as i32,
                        pixels.as_ref(),
                        &PixmapPaint::default(),
                        Transform::identity(),
                        None,
                    );
                } else {
                    let filtered = PixmapPaint {
                        quality: FilterQuality::Bilinear,
                        ..PixmapPaint::default()
                    };
                    canvas.draw_pixmap(
                        0,
                        0,
                        pixels.as_ref(),
                        &filtered,
                        Transform::from_row(width / own_w, 0.0, 0.0, height / own_h, left, top),
                        None,
                    );
                }
            }
            Mark::Layers(layers) => {
                for (path, colour) in layers {
                    paint.set_color(*colour);
                    canvas.fill_path(path, &paint, FillRule::Winding, at_pen, None);
                }
            }
        }
    }
}

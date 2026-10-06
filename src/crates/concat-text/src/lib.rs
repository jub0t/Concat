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
//! and puts the canvas's centre where the clip's centre is. The clip's offset
//! and rotation then mean the same thing for a title as for footage.
//!
//! Where the block sits on that canvas is the alignment's to say. A centred
//! title has its block centred, so the clip's position is the block's middle.
//! A left-aligned one has its block's left edge on the canvas's centre, a
//! right-aligned one its right edge: the position is the edge the words are
//! aligned to, and typing more grows the block *away* from that edge rather
//! than out from the middle - which is what left and right mean everywhere
//! else, and what a title that keeps its left margin while its words change
//! needs. [`Rendered`] reports where the block landed so a monitor can draw
//! its outline there.
//!
//! Sizes in the style are fractions of the frame's height, as the document
//! stores them, so a title looks the same at 720p and 4K. Everything here
//! converts to pixels once, at the top.

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect,
    Stroke, Transform,
};
use unicode_bidi::ParagraphBidiInfo;
use unicode_linebreak::BreakOpportunity;
use unicode_script::{Script, UnicodeScript};

/// How a title's lines sit within their block, and which point of the block
/// the clip's position holds still: its left edge, its centre or its right
/// edge. See the module docs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Align {
    /// Lines share a left edge, and the block's left edge is the anchor.
    Left,
    /// Lines are centred on each other, and the block's centre is the anchor.
    #[default]
    Center,
    /// Lines share a right edge, and the block's right edge is the anchor.
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
    /// The plate's corner radius as a fraction of frame height; zero is
    /// square.
    pub background_radius: f64,
    /// The plate's air either side of the words, as a fraction of frame
    /// height; only on an axis the style does not size.
    pub background_padding_x: f64,
    /// The same above and below.
    pub background_padding_y: f64,
    /// Baseline pitch as a multiple of the em.
    pub line_height: f64,
    /// Extra advance after every glyph, as a fraction of frame height.
    pub tracking: f64,
    /// The widest a line may run, as a fraction of frame width, before its
    /// words wrap; zero for no limit. With a limit the block is a box that
    /// wide - a word no line can hold still widens it - and the lines
    /// align inside the box rather than against each other.
    pub max_width: f64,
    /// The box's height as a fraction of frame height; zero for the words'
    /// own. With a height the block is a box that tall, at least, and the
    /// words sit centred in it.
    pub max_height: f64,
}

/// One word's box in canvas pixels, top-left origin, in reading order
/// (line by line, left to right within a line). A pixel-reveal effect
/// keys off this order rather than the word itself, which is why it is
/// exposed as rects and not text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WordRect {
    /// The box's left edge.
    pub x: i32,
    /// The box's top edge.
    pub y: i32,
    /// The box's width.
    pub width: u32,
    /// The box's height.
    pub height: u32,
}

/// The finished title as pixels: the canvas, RGBA with straight alpha,
/// for a monitor that wants it now and not from a file.
#[derive(Clone, PartialEq, Debug)]
pub struct RenderedFrame {
    /// The canvas, `width` by `height` RGBA, alpha straight.
    pub rgba: Vec<u8>,
    /// The canvas width: the frame's.
    pub width: u32,
    /// The canvas height: the frame's.
    pub height: u32,
    /// See [`Rendered::block_width`].
    pub block_width: u32,
    /// See [`Rendered::block_height`].
    pub block_height: u32,
    /// See [`Rendered::block_dx`].
    pub block_dx: i32,
    /// See [`Rendered::block_dy`].
    pub block_dy: i32,
    /// See [`Rendered::words`].
    pub words: Vec<WordRect>,
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
    /// Where the block's centre is, as an offset from the canvas's centre in
    /// pixels, x to the right. Zero for a centred title; half the block's
    /// width for a left-aligned one, whose block starts at the centre; minus
    /// that for a right-aligned one. An outline goes here, not on the clip.
    pub block_dx: i32,
    /// The vertical half of `block_dx`, y down. Always zero for now: the
    /// block is centred vertically whatever the alignment.
    pub block_dy: i32,
    /// Every word's box on the canvas, in reading order - a byproduct of
    /// layout this crate already does, kept for a caller that wants to
    /// reveal a title one word at a time without knowing what a word is.
    pub words: Vec<WordRect>,
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
    /// What titles have learnt about the faces, kept for the next one.
    cache: Mutex<Cache>,
}

/// A face's bytes: the file mapped, or the data a font was loaded from.
type FaceData = Arc<dyn AsRef<[u8]> + Send + Sync>;

#[derive(Default)]
struct Cache {
    /// Each face a title has been set in, opened once, with its index in
    /// its collection.
    data: HashMap<fontdb::ID, (FaceData, u32)>,
    /// Per character, every face that can draw it, in the database's
    /// order. Learnt a title's missing characters at a time, since each
    /// lesson reads every face; emptied when a font is added.
    covers: HashMap<char, Vec<fontdb::ID>>,
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

/// The face this build bundles, at the weights the interface uses, so a
/// title is set in the face the window is on a machine that has never
/// installed it. The window embeds the same five files (concat/ui/app.slint).
/// Licensed under the SIL Open Font License; see fonts/LICENSE-HankenGrotesk.txt.
pub const BUNDLED_FAMILY: &str = "Hanken Grotesk";
const BUNDLED: [&[u8]; 5] = [
    include_bytes!("../fonts/HankenGrotesk-Regular.ttf"),
    include_bytes!("../fonts/HankenGrotesk-Medium.ttf"),
    include_bytes!("../fonts/HankenGrotesk-SemiBold.ttf"),
    include_bytes!("../fonts/HankenGrotesk-Bold.ttf"),
    include_bytes!("../fonts/HankenGrotesk-Italic.ttf"),
];

/// The base fonts every title can use, whatever the machine has installed:
/// a working set of sans, serif, display, comic and script faces, at the
/// weights a title wants. The text presets are built on them. Each is
/// under the SIL Open Font License, Permanent Marker and Kosugi Maru
/// under Apache 2.0; the licences are beside the files in fonts/. Pixelify
/// Sans and Google Sans Code are variable fonts, drawn at their default
/// (regular) weight: fontdb sets no variation axes. Kosugi Maru carries
/// Japanese, Secular One and Rubik Scribble Hebrew. The window registers the
/// same bytes for its preset cards (concat's fonts.rs).
pub const BASE_FONTS: [&[u8]; 41] = [
    include_bytes!("../fonts/Inter-Regular.ttf"),
    include_bytes!("../fonts/Inter-SemiBold.ttf"),
    include_bytes!("../fonts/Inter-Bold.ttf"),
    include_bytes!("../fonts/Inter-Black.ttf"),
    include_bytes!("../fonts/Inter-Italic.ttf"),
    include_bytes!("../fonts/Montserrat-Regular.ttf"),
    include_bytes!("../fonts/Montserrat-Bold.ttf"),
    include_bytes!("../fonts/Montserrat-ExtraBold.ttf"),
    include_bytes!("../fonts/Montserrat-Black.ttf"),
    include_bytes!("../fonts/SpaceGrotesk-Regular.ttf"),
    include_bytes!("../fonts/SpaceGrotesk-Medium.ttf"),
    include_bytes!("../fonts/SpaceGrotesk-Bold.ttf"),
    include_bytes!("../fonts/ArchivoBlack-Regular.ttf"),
    include_bytes!("../fonts/BebasNeue-Regular.ttf"),
    include_bytes!("../fonts/Anton-Regular.ttf"),
    include_bytes!("../fonts/DMSerifDisplay-Regular.ttf"),
    include_bytes!("../fonts/DMSerifDisplay-Italic.ttf"),
    include_bytes!("../fonts/AbrilFatface-Regular.ttf"),
    include_bytes!("../fonts/Bangers-Regular.ttf"),
    include_bytes!("../fonts/PermanentMarker-Regular.ttf"),
    include_bytes!("../fonts/Pacifico-Regular.ttf"),
    include_bytes!("../fonts/Lobster-Regular.ttf"),
    include_bytes!("../fonts/CaveatBrush-Regular.ttf"),
    include_bytes!("../fonts/Staatliches-Regular.ttf"),
    include_bytes!("../fonts/BlackOpsOne-Regular.ttf"),
    include_bytes!("../fonts/GermaniaOne-Regular.ttf"),
    include_bytes!("../fonts/AtomicAge-Regular.ttf"),
    include_bytes!("../fonts/HammersmithOne-Regular.ttf"),
    include_bytes!("../fonts/SecularOne-Regular.ttf"),
    include_bytes!("../fonts/LondrinaSolid-Regular.ttf"),
    include_bytes!("../fonts/LondrinaSolid-Black.ttf"),
    include_bytes!("../fonts/BungeeHairline-Regular.ttf"),
    include_bytes!("../fonts/RubikScribble-Regular.ttf"),
    include_bytes!("../fonts/ShadowsIntoLightTwo-Regular.ttf"),
    include_bytes!("../fonts/PixelifySans-Variable.ttf"),
    include_bytes!("../fonts/GoogleSansCode-Variable.ttf"),
    include_bytes!("../fonts/KosugiMaru-Regular.ttf"),
    include_bytes!("../fonts/Frijole-Regular.ttf"),
    include_bytes!("../fonts/EmilysCandy-Regular.ttf"),
    include_bytes!("../fonts/MysteryQuest-Regular.ttf"),
    include_bytes!("../fonts/ZenTokyoZoo-Regular.ttf"),
];

/// Faces that used to be bundled and no longer are: a document that names
/// one is painted in the bundled face, not in whatever the system offers
/// for a name it does not know.
const RETIRED: [&str; 2] = ["Helvetica Neue", "Synonym"];

impl Fonts {
    /// The bundled face and the system's fonts.
    pub fn new() -> Fonts {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        for face in BUNDLED.iter().chain(BASE_FONTS.iter()) {
            db.load_font_data(face.to_vec());
        }
        Fonts {
            db,
            cache: Mutex::default(),
        }
    }

    /// Adds one font file. A file that does not parse is skipped; a title
    /// that names its family falls back to a system face.
    pub fn add_file(&mut self, path: &std::path::Path) -> bool {
        let added = self.db.load_font_file(path).is_ok();
        if added {
            // The new face may draw what nothing could before.
            self.cache
                .get_mut()
                .unwrap_or_else(|e| e.into_inner())
                .covers
                .clear();
        }
        added
    }

    /// Every family the database knows, once each, in alphabetical order
    /// without regard to case: what a font picker offers. The bundled face
    /// is among them.
    pub fn families(&self) -> Vec<String> {
        family_names(&self.db)
    }
}

/// The families a font file carries - one for a face, several for a
/// collection - or none when it is not a font this crate can read. Reads
/// the file alone, with no system font behind it.
pub fn families_in(path: &std::path::Path) -> Vec<String> {
    let mut db = fontdb::Database::new();
    if db.load_font_file(path).is_err() {
        return Vec::new();
    }
    family_names(&db)
}

fn family_names(db: &fontdb::Database) -> Vec<String> {
    let mut names: Vec<String> = db
        .faces()
        .filter_map(|face| {
            face.families
                .first()
                .map(|(name, _)| name.trim().to_owned())
        })
        .filter(|name| !name.is_empty())
        .collect();
    names.sort_by_cached_key(|name| name.to_lowercase());
    names.dedup();
    names
}

impl Fonts {
    /// The best face for a style: the named family at the nearest weight and
    /// slant, then any sans-serif, then anything at all.
    fn pick(&self, style: &TitleStyle) -> Result<fontdb::ID, Error> {
        let mut family = style
            .font_family
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        if RETIRED.contains(&family) {
            family = BUNDLED_FAMILY;
        }
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
        self.db
            .query(&query)
            .or_else(|| self.db.faces().next().map(|face| face.id))
            .ok_or(Error::NoFont)
    }

    /// The faces a title is set in and which of them sets each character.
    ///
    /// The style's own face sets everything it has a glyph for. A character
    /// it lacks is lent by another face that can draw it - one the title
    /// already borrows from if one can, so a line of Japanese is not set in
    /// three faces, else the nearest in weight and slant. What nothing can
    /// draw stays with the style's face, whose .notdef box says so.
    fn cast(&self, style: &TitleStyle) -> Result<Casting, Error> {
        let own_id = self.pick(style)?;
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let own = self.data(&mut cache, own_id).ok_or(Error::BadFont)?;
        let own_face =
            ttf_parser::Face::parse((*own.0).as_ref(), own.1).map_err(|_| Error::BadFont)?;

        let mut missing: Vec<char> = style
            .content
            .chars()
            .filter(|&ch| {
                !ch.is_control()
                    && own_face.glyph_index(ch).is_none()
                    && !cache.covers.contains_key(&ch)
            })
            .collect();
        missing.sort_unstable();
        missing.dedup();
        self.learn(&mut cache, &missing);

        let want = Want {
            weight: style.font_weight.clamp(100.0, 900.0).round() as u16,
            italic: style.italic,
        };
        let mut ids = vec![own_id];
        let paragraphs = style
            .content
            .lines()
            .map(|text| self.assign(text, &own_face, &cache, want, &mut ids))
            .collect();
        let mut faces = vec![own.clone()];
        for &id in &ids[1..] {
            // A face that will not open sets its characters in the style's
            // face instead: boxes, but in the right places.
            faces.push(self.data(&mut cache, id).unwrap_or_else(|| own.clone()));
        }
        Ok(Casting { faces, paragraphs })
    }

    /// Reads every face once for the characters in `chars`, and remembers
    /// which faces can draw each.
    fn learn(&self, cache: &mut Cache, chars: &[char]) {
        if chars.is_empty() {
            return;
        }
        for &ch in chars {
            cache.covers.insert(ch, Vec::new());
        }
        for info in self.db.faces() {
            // Apple's last resort draws every character as a labelled box:
            // a face that answers everything is no answer.
            if info
                .families
                .iter()
                .any(|(name, _)| name.contains("LastResort"))
            {
                continue;
            }
            self.db.with_face_data(info.id, |data, index| {
                let Ok(face) = ttf_parser::Face::parse(data, index) else {
                    return;
                };
                for &ch in chars {
                    if draws(&face, ch)
                        && let Some(faces) = cache.covers.get_mut(&ch)
                    {
                        faces.push(info.id);
                    }
                }
            });
        }
    }

    /// Which of `ids` sets each byte of one paragraph, adding the faces it
    /// borrows to `ids`. Index 0 is the style's own face.
    fn assign(
        &self,
        text: &str,
        own: &ttf_parser::Face<'_>,
        cache: &Cache,
        want: Want,
        ids: &mut Vec<fontdb::ID>,
    ) -> Vec<usize> {
        let mut out = vec![0; text.len()];
        let mut last = 0;
        for (at, ch) in text.char_indices() {
            let covers = cache.covers.get(&ch).map_or(&[][..], Vec::as_slice);
            let lends =
                |ids: &[fontdb::ID], index: usize| index != 0 && covers.contains(&ids[index]);
            let index = if joins(ch) && at > 0 {
                // A mark, a joiner, a skin tone: with what it modifies.
                last
            } else if own.glyph_index(ch).is_some() {
                0
            } else if lends(ids, last) {
                last
            } else if let Some(index) = (1..ids.len()).find(|&index| lends(ids, index)) {
                index
            } else if let Some(id) = self.nearest(covers, want) {
                ids.push(id);
                ids.len() - 1
            } else {
                0
            };
            out[at..at + ch.len_utf8()].fill(index);
            last = index;
        }
        out
    }

    /// Of the faces in `covers`, the one closest to the style's slant and
    /// weight; the database's order breaks a tie.
    fn nearest(&self, covers: &[fontdb::ID], want: Want) -> Option<fontdb::ID> {
        covers
            .iter()
            .filter_map(|&id| self.db.face(id))
            .min_by_key(|info| {
                let italic = info.style != fontdb::Style::Normal;
                (italic != want.italic, info.weight.0.abs_diff(want.weight))
            })
            .map(|info| info.id)
    }

    /// A face's bytes, opened once and kept.
    fn data(&self, cache: &mut Cache, id: fontdb::ID) -> Option<(FaceData, u32)> {
        if let Some(found) = cache.data.get(&id) {
            return Some(found.clone());
        }
        let info = self.db.face(id)?;
        let data: FaceData = match &info.source {
            fontdb::Source::Binary(data) | fontdb::Source::SharedFile(_, data) => data.clone(),
            fontdb::Source::File(path) => {
                let file = std::fs::File::open(path).ok()?;
                // SAFETY: a font file changed on disk while mapped would be
                // read torn - the risk fontdb takes on every system face it
                // reads, and a font being rewritten under a running editor
                // is not a case worth copying tens of megabytes to avoid.
                Arc::new(unsafe { memmap2::Mmap::map(&file) }.ok()?)
            }
        };
        cache.data.insert(id, (data.clone(), info.index));
        Some((data, info.index))
    }
}

/// The weight and slant a borrowed face should come closest to.
#[derive(Clone, Copy)]
struct Want {
    weight: u16,
    italic: bool,
}

/// A title's faces, and which sets each character; see [`Fonts::cast`].
struct Casting {
    /// Each face's bytes and index in its collection, the style's first.
    faces: Vec<(FaceData, u32)>,
    /// Per line of the content, per byte, the index of the face that sets
    /// it.
    paragraphs: Vec<Vec<usize>>,
}

/// Whether `face` can draw `ch`: it has a glyph, and the glyph is an outline.
/// A colour emoji face maps its characters to pictures with no outline,
/// which would paint nothing here, so it does not count.
fn draws(face: &ttf_parser::Face<'_>, ch: char) -> bool {
    struct Nowhere;
    impl ttf_parser::OutlineBuilder for Nowhere {
        fn move_to(&mut self, _: f32, _: f32) {}
        fn line_to(&mut self, _: f32, _: f32) {}
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn close(&mut self) {}
    }
    let Some(glyph) = face.glyph_index(ch) else {
        return false;
    };
    ch.is_whitespace() || face.outline_glyph(glyph, &mut Nowhere).is_some()
}

/// A character that belongs with the one before it and is set in its face:
/// a combining mark, a joiner or variation selector, an emoji's skin tone
/// or tag.
fn joins(ch: char) -> bool {
    ch.script() == Script::Inherited
        || ('\u{1F3FB}'..='\u{1F3FF}').contains(&ch)
        || ('\u{E0020}'..='\u{E007F}').contains(&ch)
}

/// One shaped line: its outline path in pixels, pen at the origin, its
/// advance width, and each of its words' `(start_x, end_x)` in that same
/// pen space, left to right.
struct Line {
    path: Option<Path>,
    width: f32,
    words: Vec<(f32, f32)>,
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

/// What setting a line needs: the title's faces, in [`Casting`]'s order,
/// and its sizes in pixels.
struct Setter<'a> {
    faces: &'a [rustybuzz::Face<'a>],
    em: f32,
    tracking: f32,
}

/// One line of the content, ready to be set: its text, its directions,
/// and the face each byte is set in.
struct Paragraph<'a> {
    text: &'a str,
    bidi: ParagraphBidiInfo<'a>,
    face_of: &'a [usize],
}

impl Setter<'_> {
    /// A paragraph as the lines it wraps to within `max_w` pixels. A line
    /// may end wherever Unicode's line breaking allows - at a space, or
    /// between two characters of a script that has none - and takes breaks
    /// while it fits; a stretch that fits nowhere gets a line of its own
    /// rather than being cut. No limit, one line. Space at either end of a
    /// line is not set.
    fn wrap(&self, para: &Paragraph<'_>, max_w: f32) -> Vec<Line> {
        let text = para.text;
        if text.trim().is_empty() {
            return vec![self.set(para, 0..text.len(), true)];
        }
        let trimmed = |from: usize, to: usize| from + text[from..to].trim_end().len();
        let mut line_start = text.len() - text.trim_start().len();
        if max_w <= 0.0 && !text.contains(MANDATORY) {
            return vec![self.set(para, line_start..trimmed(line_start, text.len()), true)];
        }
        let mut lines = Vec::new();
        // Where the line being built may end: the last break that fitted.
        let mut fitted: Option<usize> = None;
        for (at, kind) in unicode_linebreak::linebreaks(text) {
            if at <= line_start {
                continue;
            }
            let end = trimmed(line_start, at);
            let fits = max_w <= 0.0
                || fitted.is_none()
                || self.set(para, line_start..end, false).width <= max_w;
            if !fits && let Some(last) = fitted {
                lines.push(self.set(para, line_start..trimmed(line_start, last), true));
                line_start = last;
            }
            fitted = Some(at);
            if kind == BreakOpportunity::Mandatory {
                lines.push(self.set(para, line_start..trimmed(line_start, at), true));
                line_start = at;
                fitted = None;
            }
        }
        if lines.is_empty() {
            lines.push(self.set(para, line_start..line_start, true));
        }
        lines
    }

    /// Shapes `range` of a paragraph as one line, pen starting at (0, 0) on
    /// the baseline, and outlines it when `outline` - a line only being
    /// measured is not.
    ///
    /// The line's runs are laid left to right in the order they are seen,
    /// which for right-to-left text is not the order they are read; each
    /// run is split where its face changes, and shaped piece by piece.
    fn set(&self, para: &Paragraph<'_>, range: Range<usize>, outline: bool) -> Line {
        if range.is_empty() {
            return Line {
                path: None,
                width: 0.0,
                words: Vec::new(),
            };
        }
        let (levels, runs) = para.bidi.visual_runs(range.clone());
        let mut builder = PathBuilder::new();
        let mut pen = 0.0_f32;
        // Each glyph's cluster, as a byte into the paragraph, and the span
        // it advances over.
        let mut spans: Vec<(usize, f32, f32)> = Vec::new();
        for run in runs {
            let rtl = levels[run.start].is_rtl();
            let mut pieces = Vec::new();
            let mut start = run.start;
            for at in run.clone() {
                if para.face_of[at] != para.face_of[start] {
                    pieces.push(start..at);
                    start = at;
                }
            }
            pieces.push(start..run.end);
            // Read right to left, the piece read first is seen last.
            if rtl {
                pieces.reverse();
            }
            for piece in pieces {
                let face = &self.faces[para.face_of[piece.start]];
                let scale = self.em / face.units_per_em() as f32;
                let mut buffer = rustybuzz::UnicodeBuffer::new();
                buffer.push_str(&para.text[piece.clone()]);
                buffer.set_direction(if rtl {
                    rustybuzz::Direction::RightToLeft
                } else {
                    rustybuzz::Direction::LeftToRight
                });
                let shaped = rustybuzz::shape(face, &[], buffer);
                for (info, position) in shaped
                    .glyph_infos()
                    .iter()
                    .zip(shaped.glyph_positions().iter())
                {
                    if outline {
                        let mut outliner = Outliner {
                            builder: &mut builder,
                            scale,
                            x: pen + position.x_offset as f32 * scale,
                            y: -(position.y_offset as f32 * scale),
                        };
                        face.outline_glyph(
                            ttf_parser::GlyphId(info.glyph_id as u16),
                            &mut outliner,
                        );
                    }
                    let advance = position.x_advance as f32 * scale;
                    spans.push((piece.start + info.cluster as usize, pen, pen + advance));
                    pen += advance + self.tracking;
                }
            }
        }
        // The tracking after the last glyph is air nobody sees.
        let width = if spans.is_empty() {
            0.0
        } else {
            (pen - self.tracking).max(0.0)
        };
        let words = if outline {
            words(para.text, range)
                .into_iter()
                .filter_map(|word| {
                    spans
                        .iter()
                        .filter(|(cluster, _, _)| word.contains(cluster))
                        .fold(None, |span: Option<(f32, f32)>, &(_, left, right)| {
                            Some(span.map_or((left, right), |(l, r)| (l.min(left), r.max(right))))
                        })
                })
                .collect()
        } else {
            Vec::new()
        };
        Line {
            path: builder.finish(),
            width,
            words,
        }
    }
}

/// Line separators other than the newline the content is split at: a line
/// break inside a paragraph.
const MANDATORY: &[char] = &['\u{b}', '\u{c}', '\r', '\u{85}', '\u{2028}', '\u{2029}'];

/// The words of `range`, in reading order: the stretches between the places
/// a line may break, less their spaces. Words as a space-separated script
/// has them, and each character of one that has no spaces.
fn words(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut words = Vec::new();
    let mut start = range.start;
    for (at, _) in unicode_linebreak::linebreaks(&text[range.clone()]) {
        let at = range.start + at;
        let word = &text[start..at];
        let lead = word.len() - word.trim_start().len();
        let body = word.trim().len();
        if body > 0 {
            words.push(start + lead..start + lead + body);
        }
        start = at;
    }
    words
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
/// anchored on the canvas's centre by its alignment (see [`Align`]), and
/// returns it PNG-encoded with the block's size and where it landed.
pub fn render(
    fonts: &Fonts,
    style: &TitleStyle,
    width: u32,
    height: u32,
) -> Result<Rendered, Error> {
    let (canvas, block, words) = paint(fonts, style, width, height)?;
    let png = canvas
        .encode_png()
        .map_err(|error| Error::Encode(error.to_string()))?;
    Ok(Rendered {
        png,
        width,
        height,
        block_width: block.0,
        block_height: block.1,
        block_dx: block.2,
        block_dy: block.3,
        words,
    })
}

/// [`render`], but the pixels rather than a PNG of them: no encoding, and
/// nothing for a reader to decode again. For a monitor showing a title
/// while its words are being pulled about, where a file per pointer step
/// was the whole of the lag.
pub fn render_frame(
    fonts: &Fonts,
    style: &TitleStyle,
    width: u32,
    height: u32,
) -> Result<RenderedFrame, Error> {
    let (canvas, block, words) = paint(fonts, style, width, height)?;
    Ok(RenderedFrame {
        rgba: straight_rgba(&canvas),
        width,
        height,
        block_width: block.0,
        block_height: block.1,
        block_dx: block.2,
        block_dy: block.3,
        words,
    })
}

/// A painted block: (width, height, dx, dy), on [`Rendered`]'s terms.
type Block = (u32, u32, i32, i32);

/// The canvas with the title on it, the block, and every word's box on the
/// canvas in reading order.
fn paint(
    fonts: &Fonts,
    style: &TitleStyle,
    width: u32,
    height: u32,
) -> Result<(Pixmap, Block, Vec<WordRect>), Error> {
    let mut canvas = Pixmap::new(width, height).ok_or(Error::Canvas(width, height))?;
    let frame_h = height as f32;
    let em = (style.font_size.clamp(0.005, 1.0) as f32) * frame_h;
    let tracking = style.tracking as f32 * frame_h;
    let pitch = em * (style.line_height.max(0.5) as f32);

    let casting = fonts.cast(style)?;
    let own = {
        let (data, index) = &casting.faces[0];
        rustybuzz::Face::from_slice((**data).as_ref(), *index).ok_or(Error::BadFont)?
    };
    // A borrowed face that will not parse sets its characters in the
    // style's own: boxes, but the line still holds together.
    let faces: Vec<rustybuzz::Face<'_>> = casting
        .faces
        .iter()
        .map(|(data, index)| {
            rustybuzz::Face::from_slice((**data).as_ref(), *index).unwrap_or_else(|| own.clone())
        })
        .collect();
    // The line's metrics are the style's face's, whatever it borrows: the
    // pitch and the block should not jump because one character came from
    // elsewhere.
    let upem = own.units_per_em() as f32;
    let ascent = own.ascender() as f32 / upem * em;
    let descent = -(own.descender() as f32) / upem * em;

    // The box's limits, where the style sets them; zero is a box the words
    // size.
    let max_w = (style.max_width as f32) * width as f32;
    let max_h = (style.max_height as f32) * frame_h;
    // The plate's padding is part of the block: it is what a monitor should
    // outline, and what the title's neighbours should keep clear of. A
    // sized axis is exact - a box asked for at 400 by 120 is 400 by 120,
    // plate and all - so the air is only added on an axis the words size.
    let plate = colour(&style.background);
    let pad_x = if plate.is_some() && max_w <= 0.0 {
        (style.background_padding_x.max(0.0) as f32) * frame_h
    } else {
        0.0
    };
    let pad_y = if plate.is_some() && max_h <= 0.0 {
        (style.background_padding_y.max(0.0) as f32) * frame_h
    } else {
        0.0
    };
    // Where a line may run to. A sized box wraps at its own width. Without
    // one the words would be set on one line however long, and the canvas
    // is the frame: whatever ran past its edge was never drawn, so a long
    // title lost its end. They wrap at the frame's edge instead - the
    // whole width for a centred block, half of it for one anchored at the
    // centre and growing one way - less the plate's air on either side.
    let wrap_w = if max_w > 0.0 {
        max_w
    } else {
        let room = match style.align {
            Align::Center => width as f32,
            Align::Left | Align::Right => width as f32 / 2.0,
        };
        (room - 2.0 * pad_x).max(em)
    };

    // Shape every line with the pen at the origin; placement comes after,
    // once the block's width is known.
    let setter = Setter {
        faces: &faces,
        em,
        tracking,
    };
    let lines: Vec<Line> = style
        .content
        .lines()
        .zip(&casting.paragraphs)
        .flat_map(|(text, face_of)| {
            let para = Paragraph {
                text,
                bidi: ParagraphBidiInfo::new(text, None),
                face_of,
            };
            setter.wrap(&para, wrap_w)
        })
        .collect();
    let rows = lines.len().max(1);
    let words_w = lines.iter().map(|line| line.width).fold(0.0, f32::max);
    let words_h = (rows as f32 - 1.0) * pitch + ascent + descent;
    if words_w <= 0.0 || lines.is_empty() {
        // Nothing to paint: an empty, valid canvas.
        return Ok((canvas, (0, 0, 0, 0), Vec::new()));
    }

    // The box the words sit in. Sized by the style where the style says,
    // and by the words where it does not; never smaller than the words,
    // so a word no line can hold is still whole.
    let box_w = if max_w > 0.0 {
        max_w.max(words_w)
    } else {
        words_w
    };
    let box_h = if max_h > 0.0 {
        max_h.max(words_h)
    } else {
        words_h
    };
    let outer_w = box_w + 2.0 * pad_x;
    let outer_h = box_h + 2.0 * pad_y;
    // The anchor is the canvas's centre - where the clip's position lands -
    // and the alignment says which edge of the block sits on it, plate and
    // all. Growing words then push the far edge and leave the anchored one
    // where it is.
    let anchor_x = width as f32 / 2.0;
    let outer_left = match style.align {
        Align::Left => anchor_x,
        Align::Center => anchor_x - outer_w / 2.0,
        Align::Right => anchor_x - outer_w,
    };
    let left = outer_left + pad_x;
    // The words are centred in a box taller than they are.
    let top = (frame_h - outer_h) / 2.0 + pad_y + (box_h - words_h) / 2.0;
    let block_dx = (outer_left + outer_w / 2.0 - anchor_x).round() as i32;

    // One path for all the words, placed. Each line is aligned within the
    // box's width and sits on its own baseline. A word's box rides the
    // same indent and baseline, in reading order.
    let mut words = PathBuilder::new();
    let mut word_rects = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        let Some(path) = &line.path else { continue };
        let indent = match style.align {
            Align::Left => 0.0,
            Align::Center => (box_w - line.width) / 2.0,
            Align::Right => box_w - line.width,
        };
        let baseline = top + ascent + row as f32 * pitch;
        let placed = path
            .clone()
            .transform(Transform::from_translate(left + indent, baseline))
            .expect("a translated glyph path stays finite");
        words.push_path(&placed);
        let line_top = baseline - ascent;
        for &(start_x, end_x) in &line.words {
            word_rects.push(WordRect {
                x: (left + indent + start_x).round() as i32,
                y: line_top.round() as i32,
                width: (end_x - start_x).max(0.0).round() as u32,
                height: (ascent + descent).round() as u32,
            });
        }
    }
    let Some(words) = words.finish() else {
        return Ok((
            canvas,
            (outer_w.round() as u32, outer_h.round() as u32, block_dx, 0),
            word_rects,
        ));
    };

    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };

    // The plate, first and under everything: the whole box.
    if let Some(fill) = plate
        && let Some(rect) = Rect::from_xywh(outer_left, (frame_h - outer_h) / 2.0, outer_w, outer_h)
    {
        paint.set_color(fill);
        let radius = (style.background_radius.max(0.0) as f32) * frame_h;
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

    Ok((
        canvas,
        (outer_w.round() as u32, outer_h.round() as u32, block_dx, 0),
        word_rects,
    ))
}

/// A rectangle with rounded corners, radius clamped to half the short side.
///
/// Each corner is a cubic Bézier through the usual circle constant, which
/// is a quarter circle to within a thousandth of the radius. The corners
/// were quadratics once, with the control point at the corner itself: a
/// parabola, which bends hardest at its middle - some forty percent
/// harder than the circle there - and barely at its ends. On a plate as
/// tall as it is round, which a one-line title is once the frame is
/// portrait and the plate's air scales with its height, the two sides
/// came to visible points.
fn push_rounded_rect(builder: &mut PathBuilder, rect: Rect, radius: f32) {
    // 4/3 · (√2 − 1): where a cubic's control points sit along the
    // tangents for a quarter circle.
    const KAPPA: f32 = 0.552_284_8;
    let r = radius
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0)
        .max(0.0);
    let k = r * (1.0 - KAPPA);
    let (l, t, rgt, b) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    builder.move_to(l + r, t);
    builder.line_to(rgt - r, t);
    builder.cubic_to(rgt - k, t, rgt, t + k, rgt, t + r);
    builder.line_to(rgt, b - r);
    builder.cubic_to(rgt, b - k, rgt - k, b, rgt - r, b);
    builder.line_to(l + r, b);
    builder.cubic_to(l + k, b, l, b - k, l, b - r);
    builder.line_to(l, t + r);
    builder.cubic_to(l, t + k, l + k, t, l + r, t);
    builder.close();
}

// ── shapes ───────────────────────────────────────────────────────────────────
//
// A shape clip is a figure and its paint; the compositor wants a picture, the
// same frame-sized transparent canvas a title gets, for the same reasons (see
// the module docs). The figure is centred on the canvas, so the clip's offset
// and rotation mean what they mean for a title, and `Rendered::block_*` is
// the figure's own box, for a monitor to outline.

/// The figures a shape clip can be. Mirrors the document's own list; this
/// crate does not depend on the document.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Figure {
    /// Four equal sides.
    Square,
    /// A disc.
    Circle,
    /// Equilateral, point up.
    Triangle,
    /// A slanted rectangle, leaning right.
    Parallelogram,
    /// A rectangle narrower at the top.
    Trapezoid,
    /// A horizontal rule.
    Line,
    /// A horizontal rule with a head on its right end.
    Arrow,
}

/// Everything about a shape's look, in the document's terms: sizes are
/// fractions of the frame's height.
#[derive(Clone, PartialEq, Debug)]
pub struct ShapeStyle {
    /// Which figure.
    pub figure: Figure,
    /// The figure's longer side.
    pub size: f64,
    /// The inside's colour, `#rrggbb[aa]`; empty for an outline alone.
    pub fill: String,
    /// The outline's colour; a line or an arrow is drawn in this, falling
    /// back to `fill` when it is empty.
    pub stroke: String,
    /// The outline's thickness, and a line's or an arrow's own.
    pub stroke_width: f64,
}

/// Paints `style` centred on a `width` × `height` transparent canvas and
/// returns it PNG-encoded with the figure's box. The counterpart of
/// [`render`] for a shape.
pub fn render_shape(style: &ShapeStyle, width: u32, height: u32) -> Result<Rendered, Error> {
    let (canvas, block) = paint_shape(style, width, height)?;
    let png = canvas
        .encode_png()
        .map_err(|error| Error::Encode(error.to_string()))?;
    Ok(Rendered {
        png,
        width,
        height,
        block_width: block.0,
        block_height: block.1,
        block_dx: block.2,
        block_dy: block.3,
        words: Vec::new(),
    })
}

/// [`render_shape`], but the pixels rather than a PNG of them; the
/// counterpart of [`render_frame`].
pub fn render_shape_frame(
    style: &ShapeStyle,
    width: u32,
    height: u32,
) -> Result<RenderedFrame, Error> {
    let (canvas, block) = paint_shape(style, width, height)?;
    Ok(RenderedFrame {
        rgba: straight_rgba(&canvas),
        width,
        height,
        block_width: block.0,
        block_height: block.1,
        block_dx: block.2,
        block_dy: block.3,
        words: Vec::new(),
    })
}

/// tiny-skia keeps premultiplied pixels; a frame carries straight alpha.
fn straight_rgba(canvas: &Pixmap) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(canvas.pixels().len() * 4);
    for pixel in canvas.pixels() {
        let straight = pixel.demultiply();
        rgba.extend_from_slice(&[
            straight.red(),
            straight.green(),
            straight.blue(),
            straight.alpha(),
        ]);
    }
    rgba
}

/// The canvas with the figure on it, and the figure's box.
fn paint_shape(style: &ShapeStyle, width: u32, height: u32) -> Result<(Pixmap, Block), Error> {
    let mut canvas = Pixmap::new(width, height).ok_or(Error::Canvas(width, height))?;
    let frame_h = height as f32;
    let size = (style.size.clamp(0.01, 2.0) as f32) * frame_h;
    // The outline's thickness in pixels, never under a pixel once it is
    // asked for at all, so a hairline at 480p is still a line.
    let rule = {
        let asked = (style.stroke_width.max(0.0) as f32) * frame_h;
        if asked > 0.0 { asked.max(1.0) } else { 0.0 }
    };
    let cx = width as f32 / 2.0;
    let cy = frame_h / 2.0;

    // The figure's own box, before any outline: (w, h), centred.
    let (w, h) = match style.figure {
        Figure::Square | Figure::Circle => (size, size),
        Figure::Triangle => (size, size * 0.866),
        Figure::Parallelogram | Figure::Trapezoid => (size, size * 0.6),
        Figure::Line => (size, rule),
        Figure::Arrow => (size, (rule * 4.0).max(size * 0.18)),
    };
    let (left, top) = (cx - w / 2.0, cy - h / 2.0);
    let (right, bottom) = (left + w, top + h);

    let mut path = PathBuilder::new();
    let closed = match style.figure {
        Figure::Square => {
            path.push_rect(
                Rect::from_ltrb(left, top, right, bottom).ok_or(Error::Canvas(width, height))?,
            );
            true
        }
        Figure::Circle => {
            path.push_circle(cx, cy, size / 2.0);
            true
        }
        Figure::Triangle => {
            path.move_to(cx, top);
            path.line_to(right, bottom);
            path.line_to(left, bottom);
            path.close();
            true
        }
        Figure::Parallelogram => {
            let lean = w * 0.2;
            path.move_to(left + lean, top);
            path.line_to(right, top);
            path.line_to(right - lean, bottom);
            path.line_to(left, bottom);
            path.close();
            true
        }
        Figure::Trapezoid => {
            let inset = w * 0.2;
            path.move_to(left + inset, top);
            path.line_to(right - inset, top);
            path.line_to(right, bottom);
            path.line_to(left, bottom);
            path.close();
            true
        }
        Figure::Line => {
            path.move_to(left + rule / 2.0, cy);
            path.line_to(right - rule / 2.0, cy);
            false
        }
        Figure::Arrow => {
            // The head's arms are the box's half-height long, at 45°, so the
            // head is as tall as the box says and the shaft stops short of
            // the tip by what the stroke's round cap adds.
            let arm = h / 2.0;
            let tip = right - rule / 2.0;
            path.move_to(left + rule / 2.0, cy);
            path.line_to(tip, cy);
            path.move_to(tip - arm, cy - arm);
            path.line_to(tip, cy);
            path.line_to(tip - arm, cy + arm);
            false
        }
    };
    let Some(path) = path.finish() else {
        return Ok((canvas, (0, 0, 0, 0)));
    };

    let stroke = |colour: Color| {
        let mut paint = Paint::default();
        paint.set_color(colour);
        paint.anti_alias = true;
        paint
    };
    let line_style = |width: f32| Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };

    let fill = colour(&style.fill);
    let edge = colour(&style.stroke);
    if closed {
        if let Some(fill) = fill {
            canvas.fill_path(
                &path,
                &stroke(fill),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        if rule > 0.0
            && let Some(edge) = edge
        {
            canvas.stroke_path(
                &path,
                &stroke(edge),
                &line_style(rule),
                Transform::identity(),
                None,
            );
        }
    } else if rule > 0.0
        && let Some(ink) = edge.or(fill)
    {
        // A line or an arrow is all stroke, in the outline's colour when
        // one is set, else the fill's.
        canvas.stroke_path(
            &path,
            &stroke(ink),
            &line_style(rule),
            Transform::identity(),
            None,
        );
    }

    // The box the monitor outlines: the figure, plus the outline that
    // straddles its edge.
    let grown = if closed && rule > 0.0 && edge.is_some() {
        rule
    } else {
        0.0
    };
    let block = (
        (w + grown).ceil().max(1.0) as u32,
        (h + grown).ceil().max(1.0) as u32,
        0,
        0,
    );
    Ok((canvas, block))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_font_file_names_its_family_and_anything_else_names_none() {
        let dir = std::env::temp_dir().join(format!("concat-text-fam-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let font = dir.join("face.ttf");
        std::fs::write(&font, BUNDLED[0]).expect("written");
        assert_eq!(families_in(&font), vec![BUNDLED_FAMILY.to_owned()]);
        let words = dir.join("words.txt");
        std::fs::write(&words, b"not a font").expect("written");
        assert!(families_in(&words).is_empty());
        assert!(families_in(&dir.join("missing.otf")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_families_on_offer_are_sorted_once_each_and_include_the_bundled_face() {
        let families = Fonts::new().families();
        assert!(families.iter().any(|name| name == BUNDLED_FAMILY));
        let lowered: Vec<String> = families.iter().map(|name| name.to_lowercase()).collect();
        let mut sorted = lowered.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            lowered, sorted,
            "sorted without regard to case, and once each"
        );
    }

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
            background_radius: 0.0135,
            background_padding_x: 0.0315,
            background_padding_y: 0.018,
            line_height: 1.2,
            tracking: 0.0,
            max_width: 0.0,
            max_height: 0.0,
        }
    }

    /// A limit narrower than the words wraps them: the block comes out no
    /// wider than the limit and taller than the one-line block.
    #[test]
    fn a_width_limit_wraps_words_onto_more_lines() {
        let fonts = Fonts::new();
        let one = render(&fonts, &style("one two three four five six"), 640, 360).expect("renders");
        let mut narrow = style("one two three four five six");
        narrow.max_width = 0.3;
        let wrapped = render(&fonts, &narrow, 640, 360).expect("renders");
        assert!(
            one.block_width > wrapped.block_width,
            "{} vs {}",
            one.block_width,
            wrapped.block_width
        );
        assert!(
            wrapped.block_width <= (0.3 * 640.0) as u32 + 1,
            "{}",
            wrapped.block_width
        );
        assert!(wrapped.block_height > one.block_height);
        // and a word no line can hold still gets a line, uncut
        let mut tiny = style("unbreakable");
        tiny.max_width = 0.01;
        let rendered = render(&fonts, &tiny, 640, 360).expect("renders");
        assert!(rendered.block_width > 7);
    }

    /// A title with no box of its own wraps at the frame's edge rather than
    /// running off it: the block is never wider than the frame, and a block
    /// anchored at the centre and growing one way never wider than half.
    #[test]
    fn a_title_without_a_box_wraps_at_the_frames_edge() {
        let fonts = Fonts::new();
        let long = "the quick brown fox jumps over the lazy dog again and again and again";
        let short = render(&fonts, &style("fox"), 640, 360).expect("renders");
        let centred = render(&fonts, &style(long), 640, 360).expect("renders");
        assert!(centred.block_width <= 640, "{}", centred.block_width);
        assert!(
            centred.block_height > short.block_height,
            "{} vs {}",
            centred.block_height,
            short.block_height
        );
        let mut left = style(long);
        left.align = Align::Left;
        let left = render(&fonts, &left, 640, 360).expect("renders");
        assert!(left.block_width <= 320, "{}", left.block_width);
        // The plate's air counts: the plate stays on the canvas too.
        let mut plated = style(long);
        plated.background = "#000000".to_owned();
        let plated = render(&fonts, &plated, 640, 360).expect("renders");
        assert!(plated.block_width <= 640, "{}", plated.block_width);
    }

    /// Every word gets a box, in reading order, left to right on a line and
    /// top to bottom across lines that wrap - what a per-word reveal keys
    /// off without knowing what a word is.
    #[test]
    fn every_word_gets_a_box_in_reading_order() {
        let fonts = Fonts::new();
        let out = render(&fonts, &style("one two three"), 640, 360).expect("renders");
        assert_eq!(out.words.len(), 3, "{:?}", out.words);
        assert!(out.words[0].x < out.words[1].x);
        assert!(out.words[1].x < out.words[2].x);
        for word in &out.words {
            assert!(word.width > 0 && word.height > 0, "{word:?}");
        }

        let mut narrow = style("one two three four five six");
        narrow.max_width = 0.3;
        let wrapped = render(&fonts, &narrow, 640, 360).expect("renders");
        assert_eq!(wrapped.words.len(), 6, "{:?}", wrapped.words);
        // A later word on a lower line sits strictly below an earlier one.
        let last_row_y = wrapped.words.last().expect("six words").y;
        let first_row_y = wrapped.words.first().expect("six words").y;
        assert!(last_row_y > first_row_y, "{last_row_y} vs {first_row_y}");
    }

    fn opaque_pixels(png: &[u8]) -> usize {
        let pixmap = Pixmap::decode_png(png).expect("our own PNG decodes");
        pixmap.pixels().iter().filter(|p| p.alpha() > 0).count()
    }

    /// The leftmost and rightmost columns holding any paint.
    fn painted_span(png: &[u8]) -> (u32, u32) {
        let pixmap = Pixmap::decode_png(png).expect("our own PNG decodes");
        let width = pixmap.width();
        let mut left = width;
        let mut right = 0;
        for (index, pixel) in pixmap.pixels().iter().enumerate() {
            if pixel.alpha() > 0 {
                let x = index as u32 % width;
                left = left.min(x);
                right = right.max(x);
            }
        }
        (left, right)
    }

    /// The topmost and bottommost rows holding any paint.
    fn painted_rows(png: &[u8]) -> (u32, u32) {
        let pixmap = Pixmap::decode_png(png).expect("our own PNG decodes");
        let width = pixmap.width();
        let mut top = pixmap.height();
        let mut bottom = 0;
        for (index, pixel) in pixmap.pixels().iter().enumerate() {
            if pixel.alpha() > 0 {
                let y = index as u32 / width;
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
        (top, bottom)
    }

    // ── the box: https://github.com/jub0t/Concat/issues/119 ──

    /// A box sized by the style is exactly that size, plate and all, and
    /// the words sit centred inside it.
    #[test]
    fn a_sized_box_is_exactly_that_size_with_the_words_centred_in_it() {
        let fonts = Fonts::new();
        let mut plated = style("hi");
        plated.max_width = 0.5;
        plated.max_height = 0.4;
        plated.background = "#000000ff".to_owned();
        plated.shadow = false;
        let rendered = render(&fonts, &plated, 640, 360).expect("renders");
        assert_eq!((rendered.block_width, rendered.block_height), (320, 144));
        // The plate is the box: half the frame wide, centred, no air added.
        let (left, right) = painted_span(&rendered.png);
        assert_eq!((left, right), (160, 479), "the plate's columns");
        let (top, bottom) = painted_rows(&rendered.png);
        assert_eq!((top, bottom), (108, 251), "the plate's rows");

        // Without the plate the box is the same size, and the words are in
        // the middle of it: narrower than it, and centred on its centre.
        let mut bare = plated.clone();
        bare.background = String::new();
        let rendered = render(&fonts, &bare, 640, 360).expect("renders");
        assert_eq!((rendered.block_width, rendered.block_height), (320, 144));
        let (left, right) = painted_span(&rendered.png);
        assert!(
            left > 160 && right < 479,
            "{left}..{right} is inside the box"
        );
        let (top, bottom) = painted_rows(&rendered.png);
        let middle = f64::from(top + bottom) / 2.0;
        assert!(
            (middle - 180.0).abs() < 8.0,
            "the words are centred in the box: rows {top}..{bottom}"
        );
        assert!(top > 108 && bottom < 251, "and inside it");
    }

    /// The plate's corners follow the style's radius: square at zero, and
    /// at a large radius the corner pixel is clear while the edge between
    /// the corners is still painted.
    #[test]
    fn the_plates_corners_follow_the_radius() {
        let fonts = Fonts::new();
        let mut square = style("hi");
        square.max_width = 0.5;
        square.max_height = 0.4;
        square.background = "#000000ff".to_owned();
        square.shadow = false;
        square.background_radius = 0.0;
        // The plate spans columns 160..=479 and rows 108..=251, as the
        // sized-box test pins down: one pixel in from its top-left corner,
        // and one pixel in from the middle of its top edge.
        let probe = |png: &[u8]| {
            let pixmap = Pixmap::decode_png(png).expect("our own PNG decodes");
            let at = |x: u32, y: u32| pixmap.pixel(x, y).map(|p| p.alpha()).unwrap_or(0);
            (at(161, 109), at(320, 109))
        };
        let rendered = render(&fonts, &square, 640, 360).expect("renders");
        assert_eq!(
            probe(&rendered.png),
            (255, 255),
            "square corners are painted"
        );

        let mut round = square.clone();
        round.background_radius = 0.1;
        let rendered = render(&fonts, &round, 640, 360).expect("renders");
        let (corner, edge) = probe(&rendered.png);
        assert_eq!(corner, 0, "a rounded corner is clear");
        assert_eq!(edge, 255, "and the edge between the corners is painted");

        // And the corner is a circular arc. Its radius is 36 pixels, so
        // the top-left arc is centred on (196, 144): the pixel at (169,
        // 117) is a pixel and a half outside it, and (171, 119) a pixel
        // and a half inside. The parabola the corners once were passed
        // through the first of those and painted it.
        let pixmap = Pixmap::decode_png(&rendered.png).expect("our own PNG decodes");
        let at = |x: u32, y: u32| pixmap.pixel(x, y).map(|p| p.alpha()).unwrap_or(0);
        assert!(at(169, 117) < 32, "outside the arc: {}", at(169, 117));
        assert_eq!(at(171, 119), 255, "inside the arc");
    }

    /// The plate's air follows the style's padding: the block is the words
    /// plus twice the padding on each axis the words size, and a padding of
    /// nothing is a plate that hugs them.
    #[test]
    fn the_plates_air_follows_the_padding() {
        let fonts = Fonts::new();
        let mut hugging = style("hi");
        hugging.background = "#000000ff".to_owned();
        hugging.shadow = false;
        hugging.background_padding_x = 0.0;
        hugging.background_padding_y = 0.0;
        let tight = render(&fonts, &hugging, 640, 360).expect("renders");

        let mut roomy = hugging.clone();
        roomy.background_padding_x = 0.1;
        roomy.background_padding_y = 0.05;
        let wide = render(&fonts, &roomy, 640, 360).expect("renders");
        // 10 % of 360 either side, and 5 % above and below.
        assert_eq!(
            wide.block_width,
            tight.block_width + 72,
            "the air either side"
        );
        assert_eq!(
            wide.block_height,
            tight.block_height + 36,
            "the air above and below"
        );
    }

    /// A box narrower or shorter than the words grows to hold them: a word
    /// no line can hold is never cut, and never overflows the plate.
    #[test]
    fn a_box_smaller_than_the_words_grows_to_hold_them() {
        let fonts = Fonts::new();
        let mut tiny = style("unbreakable");
        tiny.max_width = 0.01;
        tiny.max_height = 0.01;
        tiny.background = "#000000ff".to_owned();
        tiny.shadow = false;
        let rendered = render(&fonts, &tiny, 640, 360).expect("renders");
        assert!(rendered.block_width > 6, "{}", rendered.block_width);
        assert!(rendered.block_height > 4, "{}", rendered.block_height);
        let (left, right) = painted_span(&rendered.png);
        assert!(
            right - left <= rendered.block_width,
            "nothing paints outside the box: {left}..{right} in {}",
            rendered.block_width
        );
    }

    /// A left-aligned box keeps its left edge on the anchor whatever its
    /// width, and a right-aligned one its right edge: sizing the box must
    /// not walk the words.
    #[test]
    fn a_sized_box_keeps_its_anchored_edge() {
        let fonts = Fonts::new();
        for (align, dx) in [(Align::Left, 160), (Align::Right, -160)] {
            let mut sized = style("hi");
            sized.align = align;
            sized.max_width = 0.5;
            sized.shadow = false;
            let rendered = render(&fonts, &sized, 640, 360).expect("renders");
            assert_eq!(rendered.block_width, 320);
            assert_eq!(rendered.block_dx, dx, "{align:?}");
            let (left, right) = painted_span(&rendered.png);
            match align {
                Align::Left => assert!(left >= 320 && right < 640, "{left}..{right}"),
                _ => assert!(right <= 320 && left > 0, "{left}..{right}"),
            }
        }
    }

    #[test]
    fn colours_parse_both_lengths() {
        assert_eq!(colour("#ff0000"), Some(Color::from_rgba8(255, 0, 0, 255)));
        assert_eq!(colour("#00ff0080"), Some(Color::from_rgba8(0, 255, 0, 128)));
        assert_eq!(colour(""), None);
        assert_eq!(colour("red"), None);
    }

    /// The bundled face answers by name at every weight the interface uses,
    /// upright and italic, and a document that names a retired face gets it
    /// too rather than whatever the system has.
    #[test]
    fn the_bundled_face_is_always_there_and_the_retired_names_reach_it() {
        let fonts = Fonts::new();
        let style = |family: &str, weight: f64, italic: bool| TitleStyle {
            font_family: family.to_owned(),
            font_weight: weight,
            italic,
            ..style("words")
        };
        for weight in [400.0, 500.0, 600.0, 700.0] {
            fonts
                .pick(&style(BUNDLED_FAMILY, weight, false))
                .unwrap_or_else(|_| panic!("{BUNDLED_FAMILY} at {weight}"));
        }
        fonts
            .pick(&style(BUNDLED_FAMILY, 400.0, true))
            .expect("the italic");
        let bundled = fonts
            .pick(&style(BUNDLED_FAMILY, 700.0, false))
            .expect("bold");
        for retired in RETIRED {
            let picked = fonts.pick(&style(retired, 700.0, false)).expect("a face");
            assert_eq!(picked, bundled, "{retired} is painted in the bundled face");
        }
        let quoted = fonts
            .pick(&style("\"Hanken Grotesk\"", 700.0, false))
            .expect("quotes stripped");
        assert_eq!(quoted, bundled);
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

    /// Left-aligned words start at the anchor and run right; right-aligned
    /// ones end there; centred ones straddle it. Growing the words moves
    /// only the far edge.
    #[test]
    fn alignment_anchors_the_block_on_its_edge() {
        let fonts = Fonts::new();
        let (width, height) = (640, 360);
        let centre = width / 2;
        // The shadow reaches a hair past the words to the right and below;
        // this is that hair, generously.
        let slack = 4;

        let mut left = style("Hello");
        left.align = Align::Left;
        left.shadow = false;
        let out = render(&fonts, &left, width, height).expect("renders");
        let (first, _) = painted_span(&out.png);
        assert!(
            first + slack >= centre,
            "left-aligned words start at {first}, left of the anchor"
        );
        assert!(out.block_dx > 0);
        assert!((out.block_dx as u32).abs_diff(out.block_width / 2) <= 1);
        // More words: the left edge stays, the block grows rightwards.
        let mut longer = left.clone();
        longer.content = "Hello there".to_owned();
        let more = render(&fonts, &longer, width, height).expect("renders");
        let (again, _) = painted_span(&more.png);
        assert!(
            again.abs_diff(first) <= 1,
            "the left edge moved from {first} to {again}"
        );
        assert!(more.block_width > out.block_width);

        let mut right = style("Hello");
        right.align = Align::Right;
        right.shadow = false;
        let out = render(&fonts, &right, width, height).expect("renders");
        let (_, last) = painted_span(&out.png);
        assert!(
            last <= centre + slack,
            "right-aligned words end at {last}, right of the anchor"
        );
        assert!(out.block_dx < 0);

        let out = render(&fonts, &style("Hello"), width, height).expect("renders");
        let (first, last) = painted_span(&out.png);
        assert!(first < centre && last > centre);
        assert_eq!(out.block_dx, 0);
    }

    // ── scripts the style's face lacks ──

    /// A character the style's face has no glyph for is set in a face that
    /// can draw it, when the machine has one - not in the style's .notdef
    /// box. Skipped on a machine with no face for it.
    #[test]
    fn a_character_the_face_lacks_is_borrowed_from_one_that_has_it() {
        let fonts = Fonts::new();
        let mut han = style("漢字 and words");
        han.font_family = BUNDLED_FAMILY.to_owned();
        let casting = fonts.cast(&han).expect("casts");
        let face_of = &casting.paragraphs[0];
        // The Latin stays in the style's face.
        assert_eq!(face_of[han.content.find('a').expect("an a")], 0);
        if casting.faces.len() == 1 {
            eprintln!("no face on this machine draws 漢; skipped");
            return;
        }
        let (data, index) = &casting.faces[face_of[0]];
        let lent = ttf_parser::Face::parse((**data).as_ref(), *index).expect("parses");
        assert!(draws(&lent, '漢') && draws(&lent, '字'));
        assert_eq!(face_of[0], face_of['漢'.len_utf8()], "one face for the run");
    }

    /// A combining mark is set in the face of the character it sits on,
    /// whichever face that is, so the two are shaped together.
    #[test]
    fn a_mark_is_set_with_what_it_marks() {
        let fonts = Fonts::new();
        let content = "漢\u{301}e\u{301}";
        let casting = fonts.cast(&style(content)).expect("casts");
        let face_of = &casting.paragraphs[0];
        let mark = '漢'.len_utf8();
        assert_eq!(face_of[mark], face_of[0]);
        let second = mark + '\u{301}'.len_utf8() + 1;
        assert_eq!(face_of[second], face_of[second - 1]);
    }

    /// Right-to-left words are laid right to left: the first read is the
    /// rightmost. The order holds whatever face draws them.
    #[test]
    fn right_to_left_words_run_right_to_left() {
        let fonts = Fonts::new();
        let out = render(&fonts, &style("שלום עולם"), 640, 360).expect("renders");
        assert_eq!(out.words.len(), 2, "{:?}", out.words);
        assert!(
            out.words[0].x > out.words[1].x,
            "the first word read is on the right: {:?}",
            out.words
        );
    }

    /// A script without spaces still wraps under a width limit, between its
    /// characters, and each character is a word of its own.
    #[test]
    fn a_script_without_spaces_wraps_between_its_characters() {
        let fonts = Fonts::new();
        let content = "漢字漢字漢字漢字漢字漢字";
        let one = render(&fonts, &style(content), 640, 360).expect("renders");
        let mut narrow = style(content);
        narrow.max_width = 0.2;
        let wrapped = render(&fonts, &narrow, 640, 360).expect("renders");
        assert!(wrapped.block_height > one.block_height, "it wrapped");
        assert_eq!(wrapped.words.len(), content.chars().count());
    }

    /// A line separator inside a paragraph breaks the line.
    #[test]
    fn a_line_separator_breaks_the_line() {
        let fonts = Fonts::new();
        let one = render(&fonts, &style("One Two"), 640, 360).expect("renders");
        let two = render(&fonts, &style("One\u{2028}Two"), 640, 360).expect("renders");
        assert!(two.block_height > one.block_height);
        assert!(two.block_width < one.block_width);
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

    /// A shape is painted centred, at the size the style says, and a rule
    /// is all stroke in the fill's colour when no outline colour is given.
    #[test]
    fn a_shape_is_painted_centred_at_its_size() {
        let square = ShapeStyle {
            figure: Figure::Square,
            size: 0.5,
            fill: "#ff0000".to_owned(),
            stroke: String::new(),
            stroke_width: 0.0,
        };
        let frame = render_shape_frame(&square, 400, 200).expect("paints");
        assert_eq!((frame.block_width, frame.block_height), (100, 100));
        let px = |x: u32, y: u32| {
            let at = ((y * 400 + x) * 4) as usize;
            (frame.rgba[at], frame.rgba[at + 3])
        };
        assert_eq!(px(200, 100), (255, 255), "the centre is red and solid");
        assert_eq!(px(140, 100).1, 0, "ten pixels past the edge is clear");
        assert_eq!(px(10, 10).1, 0, "and so is the corner");

        let line = ShapeStyle {
            figure: Figure::Line,
            size: 0.5,
            fill: "#00ff00".to_owned(),
            stroke: String::new(),
            stroke_width: 0.05,
        };
        let frame = render_shape_frame(&line, 400, 200).expect("paints");
        assert_eq!((frame.block_width, frame.block_height), (100, 10));
        let at = ((100 * 400 + 200) * 4) as usize;
        assert_eq!(
            (frame.rgba[at + 1], frame.rgba[at + 3]),
            (255, 255),
            "the rule runs through the centre in the fill's green"
        );
        let at = ((100 * 400 + 140) * 4) as usize;
        assert_eq!(frame.rgba[at + 3], 0, "and stops at its length");

        let png = render_shape(&square, 64, 64).expect("encodes");
        assert!(png.png.starts_with(b"\x89PNG"));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Titles as pixels.
//!
//! A text clip is a style and some words; the compositor wants a picture. This
//! crate is the step between: it finds the face, shapes each line, turns the
//! glyph outlines into paths, and paints them - plate, shadow, outline, fill,
//! and the glyphs that paint their own colours - onto a canvas the size of
//! the output frame, transparent everywhere the words are not.
//!
//! Frame-sized on purpose. The compositor places a picture by fitting it into
//! the frame and then applying the clip's transform about its centre, so a
//! canvas that *is* the frame fits at exactly one, decodes without resampling,
//! and puts the canvas's centre where the clip's centre is. The clip's offset
//! and rotation then mean the same thing for a title as for footage.
//!
//! Frame-sized at the least, that is. Ink past the frame's edges - a box
//! pulled wider than the frame, a word too long for any line, the outline,
//! shadow or lean of letters on a line as wide as the frame - would be cut
//! off at the canvas's edge, and a title scaled down, moved or turned
//! showed the cut. The canvas grows instead, by the same margin either
//! side, so its centre stays the clip's centre; [`Rendered::overscan`] says
//! by how much, for the compositor to draw it at one pixel to one rather
//! than fit it into the frame.
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
    Color, FillRule, FilterQuality, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap,
    PixmapPaint, Rect, Stroke, Transform,
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
    /// Decorations share the glyph fill, outline and shadow.
    pub underline: bool,
    /// A rule through each line using the selected face's strike metrics.
    pub strikethrough: bool,
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
    /// Stretches of the words in a look of their own over the rest; see
    /// [`TitleRun`]. Empty for a title in one look.
    pub runs: Vec<TitleRun>,
}

/// Some of a title's words in a look of their own: the characters
/// `start..end` of the content - characters, not bytes - and the fields
/// they wear instead of the title's. A field left out is the title's. The
/// document's run, field for field.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct TitleRun {
    /// The first character.
    pub start: usize,
    /// One past the last.
    pub end: usize,
    /// Fill colour as `#rrggbb[aa]`.
    pub color: Option<String>,
    /// CSS-scale weight; the face is picked for it, as for the title's.
    pub font_weight: Option<f64>,
    /// Italic or upright.
    pub italic: Option<bool>,
    /// Underlined or not.
    pub underline: Option<bool>,
    /// Struck through or not.
    pub strikethrough: Option<bool>,
    /// The em as a multiple of the title's; a line with a bigger word in
    /// it is as tall as that word.
    pub font_scale: Option<f64>,
    /// Outline colour, where the title has an outline.
    pub stroke_color: Option<String>,
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
    /// See [`Rendered::width`].
    pub width: u32,
    /// See [`Rendered::height`].
    pub height: u32,
    /// See [`Rendered::overscan`].
    pub overscan: f64,
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
    /// The canvas width: the frame's, and more where the title's ink runs
    /// past the frame's left or right edge - the same margin either side.
    pub width: u32,
    /// The canvas height: the frame's, grown the same way for ink past the
    /// top or the bottom.
    pub height: u32,
    /// How many times the frame the canvas is, along the axis it grew the
    /// most: one for a title inside the frame, which is nearly every title.
    /// A picture this many times the frame is drawn at one canvas pixel to
    /// one frame pixel, where the compositor would otherwise fit the whole
    /// canvas into the frame.
    pub overscan: f64,
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
    /// Per character, the faces that can draw it; learnt a title's missing
    /// characters at a time, since each lesson reads every face; emptied
    /// when a font is added.
    covers: HashMap<char, Covers>,
}

/// The faces that can draw one character: those that paint colours of their
/// own first, then the rest in the database's order. An emoji is wanted in
/// colour whatever the title's weight, so a monochrome face that also has
/// it - DejaVu, Symbola, hundreds of glyphs each - is only reached when no
/// colour face does.
#[derive(Default)]
struct Covers {
    colour: Vec<fontdb::ID>,
    plain: Vec<fontdb::ID>,
}

impl Covers {
    /// Whether `id` is one of them.
    fn has(&self, id: fontdb::ID) -> bool {
        self.colour.contains(&id) || self.plain.contains(&id)
    }
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

/// Noto Sans CJK SC, cut to GB 2312, Big5 level 1, JIS X 0208, KS X 1001's
/// Hangul, the kana, the CJK punctuation and fullwidth forms, and every
/// character in the shipped locales. Under three megabytes rather than
/// sixteen. Japanese and Traditional Chinese read in the Simplified face's
/// forms where the regions differ; a machine's own font for them comes
/// first.
pub const NOTO_SANS_CJK_SC: &[u8] = include_bytes!("../fonts/NotoSansCJKsc-Concat.otf");

/// Noto Sans Arabic UI 2.011, for Arabic, Persian and Urdu: the UI cut,
/// whose ascent and descent fit a line of Latin, where Noto Sans Arabic's
/// would stand taller than the rows it is put in. Urdu reads in its
/// Naskh-like forms, not Nastaliq; every Urdu letter is there. From
/// https://github.com/notofonts/arabic/releases/download/NotoSansArabicUI-v2.011/NotoSansArabicUI-v2.011.zip
/// (SHA-256 f775ee259557195721466e99ec0316bbcd897f0429e25955f5d9582ca1202ce7).
pub const NOTO_SANS_ARABIC_UI: &[u8] = include_bytes!("../fonts/NotoSansArabicUI-Concat.ttf");

/// Noto Sans Hebrew 3.001. From
/// https://github.com/notofonts/hebrew/releases/download/NotoSansHebrew-v3.001/NotoSansHebrew-v3.001.zip
/// (SHA-256 df0a71814b4e63644cf40fcc4529111b61266b7a2dafbe95068b29a7520cc3cb).
pub const NOTO_SANS_HEBREW: &[u8] = include_bytes!("../fonts/NotoSansHebrew-Concat.ttf");

/// Noto Sans Devanagari 2.007, for Hindi. From
/// https://github.com/notofonts/devanagari/releases/download/NotoSansDevanagari-v2.007/NotoSansDevanagari-v2.007.zip
/// (SHA-256 820c7da45b1e63562cb41c0a8cac5d9a4202312043a3a040ed1325857ef469b1).
pub const NOTO_SANS_DEVANAGARI: &[u8] = include_bytes!("../fonts/NotoSansDevanagari-Concat.ttf");

/// Noto Sans Bengali 3.011. From
/// https://github.com/notofonts/bengali/releases/download/NotoSansBengali-v3.011/NotoSansBengali-v3.011.zip
/// (SHA-256 2aca24bf71665e66c32e14862ee58857dd8b8b07b6321ec7c96f3121ad6683b1).
pub const NOTO_SANS_BENGALI: &[u8] = include_bytes!("../fonts/NotoSansBengali-Concat.ttf");

/// Noto Sans Thai 2.002, the loopless face Thai interfaces are set in.
/// From https://github.com/notofonts/thai/releases/download/NotoSansThai-v2.002/NotoSansThai-v2.002.zip
/// (SHA-256 af889cc673fc714060ce5e4e088fbad32aa4c0571a19958efeaff128a22da485).
pub const NOTO_SANS_THAI: &[u8] = include_bytes!("../fonts/NotoSansThai-Concat.ttf");

/// A face for each script the base fonts have no glyphs for, which every
/// title falls back on after the machine's own fonts: a caption in Thai,
/// Arabic or Korean draws its words wherever it is painted, not boxes on a
/// phone with no font of its own to lend. Each is the static Regular
/// instance from the notofonts releases, unhinted, cut with fonttools'
/// `pyftsubset` to the script's whole basic block, Latin-1, the general
/// punctuation, U+25CC (the dotted circle the shaper puts under a mark
/// with nothing to sit on) and every character the shipped locales use,
/// with every layout feature kept so the joining and conjuncts still shape:
///
/// ```text
/// pyftsubset <face>-Regular.ttf --unicodes=<block>,U+0020-007E,U+00A0-00FF,U+2000-206F,U+25CC \
///     --text-file=<every locale's characters> --layout-features='*' --no-hinting \
///     --notdef-outline --name-IDs=0,1,2,3,4,5,6,13,14 --output-file=<face>-Concat.ttf
/// ```
///
/// The licences are `fonts/LICENSE-<face>.txt`. The window registers the
/// same bytes as its interface's fallbacks (concat's fonts.rs).
pub const SCRIPT_FALLBACKS: [&[u8]; 6] = [
    NOTO_SANS_CJK_SC,
    NOTO_SANS_ARABIC_UI,
    NOTO_SANS_HEBREW,
    NOTO_SANS_DEVANAGARI,
    NOTO_SANS_BENGALI,
    NOTO_SANS_THAI,
];

/// Faces that used to be bundled and no longer are: a document that names
/// one is painted in the bundled face, not in whatever the system offers
/// for a name it does not know.
const RETIRED: [&str; 2] = ["Helvetica Neue", "Synonym"];

impl Fonts {
    /// The bundled faces, the system's fonts, and a face for every script
    /// the bundled ones lack.
    pub fn new() -> Fonts {
        Fonts::with_system(true)
    }

    /// The faces titles can use, the machine's own among them or not. The
    /// machine's come first, so where it has a face for a script, at the
    /// title's weight, that face draws it; the script fallbacks come last.
    fn with_system(system: bool) -> Fonts {
        let mut db = fontdb::Database::new();
        if system {
            db.load_system_fonts();
            // fontdb knows a desk's font folders and not a phone's, so on a
            // phone the system had lent titles nothing: every script the
            // bundled faces lack was boxes.
            #[cfg(target_os = "android")]
            for dir in ["/system/fonts", "/product/fonts"] {
                db.load_fonts_dir(dir);
            }
            #[cfg(target_os = "ios")]
            db.load_fonts_dir("/System/Library/Fonts");
        }
        for face in BUNDLED.iter().chain(BASE_FONTS.iter()) {
            db.load_font_data(face.to_vec());
        }
        for face in SCRIPT_FALLBACKS {
            db.load_font_source(fontdb::Source::Binary(Arc::new(face)));
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
    #[cfg(test)]
    fn pick(&self, style: &TitleStyle) -> Result<fontdb::ID, Error> {
        self.pick_face(&style.font_family, style.font_weight, style.italic)
    }

    /// [`Fonts::pick`] for one look of a title: its family, at a weight
    /// and slant that may be a run's rather than the title's.
    fn pick_face(&self, family: &str, weight: f64, italic: bool) -> Result<fontdb::ID, Error> {
        let mut family = family.trim().trim_matches('"').trim_matches('\'');
        if RETIRED.contains(&family) {
            family = BUNDLED_FAMILY;
        }
        let weight = fontdb::Weight(weight.clamp(100.0, 900.0).round() as u16);
        let slant = if italic {
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
    #[cfg(test)]
    fn cast(&self, style: &TitleStyle) -> Result<Casting, Error> {
        let (looks, look_of) = looks_of(style);
        self.cast_looks(style, &looks, &look_of)
    }

    /// The faces a title is set in and which of them sets each character,
    /// for its looks (see [`Look`]) and the look of each byte.
    ///
    /// Each look's own face - the family at that look's weight and slant,
    /// the title's own first - sets everything it has a glyph for. A
    /// character it lacks is lent by another face that can draw it - one
    /// the title already borrows from if one can, so a line of Japanese is
    /// not set in three faces, else the nearest in weight and slant. What
    /// nothing can draw stays with the look's face, whose .notdef box says
    /// so.
    fn cast_looks(
        &self,
        style: &TitleStyle,
        looks: &[Look],
        look_of: &[usize],
    ) -> Result<Casting, Error> {
        // Each look's own face, once each however many looks share it; the
        // title's is first, so index 0 is always the style's face.
        let mut ids: Vec<fontdb::ID> = Vec::new();
        let mut owns = Vec::with_capacity(looks.len());
        for look in looks {
            let id = self.pick_face(&style.font_family, look.weight, look.italic)?;
            owns.push(match ids.iter().position(|&held| held == id) {
                Some(index) => index,
                None => {
                    ids.push(id);
                    ids.len() - 1
                }
            });
        }
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let own = self.data(&mut cache, ids[0]).ok_or(Error::BadFont)?;
        // A run's face that will not open sets its characters in the
        // title's: the right weight lost, the words still there.
        let own_data: Vec<(FaceData, u32)> = std::iter::once(own.clone())
            .chain(
                ids[1..]
                    .iter()
                    .map(|&id| self.data(&mut cache, id).unwrap_or_else(|| own.clone())),
            )
            .collect();
        let title_face =
            ttf_parser::Face::parse((*own.0).as_ref(), own.1).map_err(|_| Error::BadFont)?;
        let own_faces: Vec<ttf_parser::Face<'_>> = own_data
            .iter()
            .map(|(data, index)| {
                ttf_parser::Face::parse((**data).as_ref(), *index)
                    .unwrap_or_else(|_| title_face.clone())
            })
            .collect();
        let own_of = |at: usize| owns[look_of.get(at).copied().unwrap_or(0)];

        let mut missing: Vec<char> = style
            .content
            .char_indices()
            .filter(|&(at, ch)| {
                !ch.is_control()
                    && own_faces[own_of(at)].glyph_index(ch).is_none()
                    && !cache.covers.contains_key(&ch)
            })
            .map(|(_, ch)| ch)
            .collect();
        missing.sort_unstable();
        missing.dedup();
        self.learn(&mut cache, &missing);

        let paragraphs = style
            .content
            .lines()
            .map(|text| {
                let at = text.as_ptr() as usize - style.content.as_ptr() as usize;
                let looks_here = look_of.get(at..at + text.len()).unwrap_or(&[]);
                self.assign(text, looks_here, looks, &owns, &own_faces, &cache, &mut ids)
            })
            .collect();
        let mut faces = own_data;
        for &id in &ids[faces.len()..] {
            // A face that will not open sets its characters in the style's
            // face instead: boxes, but in the right places.
            faces.push(self.data(&mut cache, id).unwrap_or_else(|| own.clone()));
        }
        Ok(Casting {
            faces,
            paragraphs,
            owns,
        })
    }

    /// Reads every face once for the characters in `chars`, and remembers
    /// which faces can draw each.
    fn learn(&self, cache: &mut Cache, chars: &[char]) {
        if chars.is_empty() {
            return;
        }
        for &ch in chars {
            cache.covers.insert(ch, Covers::default());
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
                    let (Some(how), Some(covers)) = (draws(&face, ch), cache.covers.get_mut(&ch))
                    else {
                        continue;
                    };
                    match how {
                        Draw::Colour => covers.colour.push(info.id),
                        Draw::Outline => covers.plain.push(info.id),
                    }
                }
            });
        }
    }

    /// Which of `ids` sets each byte of one paragraph, adding the faces it
    /// borrows to `ids`. `look_of` is the look of each of its bytes, and
    /// `owns` each look's own face in `ids`, parsed in `own_faces`.
    #[allow(clippy::too_many_arguments)]
    fn assign(
        &self,
        text: &str,
        look_of: &[usize],
        looks: &[Look],
        owns: &[usize],
        own_faces: &[ttf_parser::Face<'_>],
        cache: &Cache,
        ids: &mut Vec<fontdb::ID>,
    ) -> Vec<usize> {
        let mut out = vec![0; text.len()];
        let mut last = 0;
        for (at, ch) in text.char_indices() {
            let look = look_of.get(at).copied().unwrap_or(0);
            let own = owns[look];
            let want = Want {
                weight: looks[look].weight.clamp(100.0, 900.0).round() as u16,
                italic: looks[look].italic,
            };
            let covers = cache.covers.get(&ch);
            let lends = |ids: &[fontdb::ID], index: usize| {
                index != own && covers.is_some_and(|covers| covers.has(ids[index]))
            };
            let index = if joins(ch) && at > 0 {
                // A mark, a joiner, a skin tone: with what it modifies.
                last
            } else if own_faces[own].glyph_index(ch).is_some() {
                own
            } else if lends(ids, last) {
                last
            } else if let Some(index) = (0..ids.len()).find(|&index| lends(ids, index)) {
                index
            } else if let Some(id) = self.nearest(covers, want) {
                ids.push(id);
                ids.len() - 1
            } else {
                own
            };
            out[at..at + ch.len_utf8()].fill(index);
            last = index;
        }
        out
    }

    /// Of the faces that cover the character, the one closest to the style's
    /// slant and weight; the database's order breaks a tie. A colour face
    /// wins before weight is considered: its colours are the point of it,
    /// and a title in a heavy weight still wants its emoji in colour.
    fn nearest(&self, covers: Option<&Covers>, want: Want) -> Option<fontdb::ID> {
        let covers = covers?;
        let closest = |ids: &[fontdb::ID]| {
            ids.iter()
                .filter_map(|&id| self.db.face(id))
                .min_by_key(|info| {
                    let italic = info.style != fontdb::Style::Normal;
                    (italic != want.italic, info.weight.0.abs_diff(want.weight))
                })
                .map(|info| info.id)
        };
        closest(&covers.colour).or_else(|| closest(&covers.plain))
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

/// A title's faces, and which sets each character; see [`Fonts::cast_looks`].
struct Casting {
    /// Each face's bytes and index in its collection: each look's own
    /// first, the style's at 0, then the ones borrowed.
    faces: Vec<(FaceData, u32)>,
    /// Per line of the content, per byte, the index of the face that sets
    /// it.
    paragraphs: Vec<Vec<usize>>,
    /// Per look, the index of its own face.
    owns: Vec<usize>,
}

/// One look a title's characters are set in, resolved: the title's own,
/// at index 0, or a run's laid over it - each field the run's where it
/// says and the title's where it does not.
#[derive(Clone, Debug)]
struct Look {
    weight: f64,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    /// Its em, as a multiple of the title's.
    scale: f32,
    fill: Color,
    /// The outline's colour; none for one that does not parse, which the
    /// title's outline did not draw either.
    edge: Option<Color>,
}

/// The looks a title is set in - its own first, then one per run - and,
/// per byte of the content, which. A title with no runs is one look, and
/// is set exactly as it was before runs.
fn looks_of(style: &TitleStyle) -> (Vec<Look>, Vec<usize>) {
    let base = Look {
        weight: style.font_weight,
        italic: style.italic,
        underline: style.underline,
        strikethrough: style.strikethrough,
        scale: 1.0,
        fill: colour(&style.color).unwrap_or(Color::WHITE),
        edge: colour(&style.stroke_color),
    };
    let mut look_of = vec![0; style.content.len()];
    let mut looks = vec![base.clone()];
    if style.runs.is_empty() {
        return (looks, look_of);
    }
    // Characters to bytes: a run counts characters, the setter bytes.
    let bytes: Vec<usize> = style
        .content
        .char_indices()
        .map(|(at, _)| at)
        .chain(std::iter::once(style.content.len()))
        .collect();
    let count = bytes.len() - 1;
    for run in &style.runs {
        let (start, end) = (run.start.min(count), run.end.min(count));
        if start >= end {
            continue;
        }
        looks.push(Look {
            weight: run.font_weight.unwrap_or(base.weight),
            italic: run.italic.unwrap_or(base.italic),
            underline: run.underline.unwrap_or(base.underline),
            strikethrough: run.strikethrough.unwrap_or(base.strikethrough),
            scale: run
                .font_scale
                .filter(|scale| scale.is_finite())
                .map_or(1.0, |scale| scale.clamp(0.1, 10.0) as f32),
            fill: run.color.as_deref().and_then(colour).unwrap_or(base.fill),
            edge: run.stroke_color.as_deref().and_then(colour).or(base.edge),
        });
        look_of[bytes[start]..bytes[end]].fill(looks.len() - 1);
    }
    (looks, look_of)
}

/// What `face` draws `ch` with, when it draws it: colours of its own, or
/// an outline for the title's fill to colour.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Draw {
    /// A bitmap strike or layered `COLR` paints - a picture the face
    /// colours itself, as an emoji is.
    Colour,
    /// An outline the words' fill colours.
    Outline,
}

/// Whether `face` can draw `ch`, and how: it has a glyph, and the glyph
/// either carries colours the face paints on its own - a colour emoji
/// face's picture, which has no outline to fill - or is an outline.
fn draws(face: &ttf_parser::Face<'_>, ch: char) -> Option<Draw> {
    struct Nowhere;
    impl ttf_parser::OutlineBuilder for Nowhere {
        fn move_to(&mut self, _: f32, _: f32) {}
        fn line_to(&mut self, _: f32, _: f32) {}
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn close(&mut self) {}
    }
    let glyph = face.glyph_index(ch)?;
    if own_colours(face, glyph) {
        return Some(Draw::Colour);
    }
    (ch.is_whitespace() || face.outline_glyph(glyph, &mut Nowhere).is_some())
        .then_some(Draw::Outline)
}

/// Whether `glyph` paints colours of its own that this crate can draw: a
/// bitmap strike in a colour format, or the layered paints of a `COLR`
/// face simple enough to follow - solid fills over outlines, which is all
/// a layered emoji face needs. A monochrome strike is not one; that is
/// the outline's job, done better. An `SVG` document would want another
/// renderer whole, and stays out. https://github.com/jub0t/Concat/issues/276
fn own_colours(face: &ttf_parser::Face<'_>, glyph: ttf_parser::GlyphId) -> bool {
    if let Some(image) = face.glyph_raster_image(glyph, u16::MAX)
        && matches!(
            image.format,
            ttf_parser::RasterImageFormat::PNG | ttf_parser::RasterImageFormat::BitmapPremulBgra32
        )
    {
        return true;
    }
    if !face.is_color_glyph(glyph) {
        return false;
    }
    // Probed rather than trusted: a `COLR` v1 glyph drawn with gradients
    // or clips would come out wrong, and a face that only has those would
    // draw nothing at all - a box, or a fallback face, beats that.
    Layering::of(face, glyph, ttf_parser::RgbaColor::new(0, 0, 0, 255)).is_some()
}

/// What a face's `COLR` says about one glyph: the layers it paints, as
/// glyph and palette colour, in the order they go down - and whether it
/// painted only what this crate follows. A gradient, a clip, a transform
/// or a composite mode would come out wrong or not at all, so the glyph
/// falls back to its outline instead.
struct Layering {
    layers: Vec<(ttf_parser::GlyphId, ttf_parser::RgbaColor)>,
    /// The outline awaiting its colour; `COLR` pairs them.
    pending: Option<ttf_parser::GlyphId>,
    /// Something outside the simple case turned up.
    other: bool,
}

impl Layering {
    /// The layers `COLR` paints for `glyph`, or `None` when it paints none
    /// or paints them in a way this crate does not follow.
    fn of(
        face: &ttf_parser::Face<'_>,
        glyph: ttf_parser::GlyphId,
        foreground: ttf_parser::RgbaColor,
    ) -> Option<Vec<(ttf_parser::GlyphId, ttf_parser::RgbaColor)>> {
        let mut layering = Layering {
            layers: Vec::new(),
            pending: None,
            other: false,
        };
        face.paint_color_glyph(glyph, 0, foreground, &mut layering)?;
        (!layering.other && layering.pending.is_none() && !layering.layers.is_empty())
            .then_some(layering.layers)
    }
}

impl<'a> ttf_parser::colr::Painter<'a> for Layering {
    fn outline_glyph(&mut self, glyph_id: ttf_parser::GlyphId) {
        self.pending = Some(glyph_id);
    }
    fn paint(&mut self, paint: ttf_parser::colr::Paint<'a>) {
        match (paint, self.pending.take()) {
            (ttf_parser::colr::Paint::Solid(colour), Some(glyph_id)) => {
                self.layers.push((glyph_id, colour));
            }
            _ => self.other = true,
        }
    }
    fn push_clip(&mut self) {
        self.other = true;
    }
    fn push_clip_box(&mut self, _: ttf_parser::colr::ClipBox) {
        self.other = true;
    }
    fn pop_clip(&mut self) {
        self.other = true;
    }
    fn push_layer(&mut self, _: ttf_parser::colr::CompositeMode) {
        self.other = true;
    }
    fn pop_layer(&mut self) {
        self.other = true;
    }
    fn push_transform(&mut self, _: ttf_parser::Transform) {
        self.other = true;
    }
    fn pop_transform(&mut self) {
        self.other = true;
    }
}

/// A character that belongs with the one before it and is set in its face:
/// a combining mark, a joiner or variation selector, an emoji's skin tone
/// or tag.
fn joins(ch: char) -> bool {
    ch.script() == Script::Inherited
        || ('\u{1F3FB}'..='\u{1F3FF}').contains(&ch)
        || ('\u{E0020}'..='\u{E007F}').contains(&ch)
}

/// One shaped line: its outlines in pixels, pen at the origin, one path
/// per look that has glyphs on it, its advance width, and each of its
/// words' `(start_x, end_x)` in that same pen space, left to right.
struct Line {
    /// `(look, outlines)`, for the look's fill and outline to paint.
    paths: Vec<(usize, Path)>,
    /// The glyphs that paint their own colours - a picture rather than an
    /// outline - at that same pen origin, for laying over the words. The
    /// fill would wash their colours out, so they stay out of `paths`.
    marks: Vec<Mark>,
    width: f32,
    words: Vec<(f32, f32)>,
    /// The stretches set in one look, `(look, start_x, end_x)` left to
    /// right: where a look's underline and strike are ruled.
    stretches: Vec<(usize, f32, f32)>,
    /// The tallest of its looks' ascents and descents, and its biggest em,
    /// in pixels: a line with one big word is as tall as that word.
    ascent: f32,
    descent: f32,
    em: f32,
}

/// A glyph that colours itself, in pen space at its line's origin: an
/// emoji, or a layered `COLR` face's glyphs. Drawn after the words, with
/// no shadow and no outline of their own - they are pictures, and a box
/// around them would be a box. https://github.com/jub0t/Concat/issues/276
enum Mark {
    /// The font's own bitmap, decoded, at its box: left edge, top edge,
    /// and size in pixels.
    Bitmap {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        pixmap: Pixmap,
    },
    /// Outlines each with the colour the face paints it in, already
    /// placed.
    Layers(Vec<(Path, Color)>),
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

/// The same colour in the bytes a font's palette wants.
fn rgba(color: Color) -> ttf_parser::RgbaColor {
    ttf_parser::RgbaColor::new(
        (color.red() * 255.0).round() as u8,
        (color.green() * 255.0).round() as u8,
        (color.blue() * 255.0).round() as u8,
        (color.alpha() * 255.0).round() as u8,
    )
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
/// its looks, and its sizes in pixels.
struct Setter<'a> {
    faces: &'a [rustybuzz::Face<'a>],
    /// The title's em; a look's is this times its scale.
    em: f32,
    tracking: f32,
    looks: &'a [Look],
    /// Per look, its own face's ascent and descent at the look's size.
    metrics: &'a [(f32, f32)],
}

/// One line of the content, ready to be set: its text, its directions,
/// and the face and the look each byte is set in.
struct Paragraph<'a> {
    text: &'a str,
    bidi: ParagraphBidiInfo<'a>,
    face_of: &'a [usize],
    look_of: &'a [usize],
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

    /// How tall a line of `range` stands: the tallest ascent and descent
    /// of the looks on it, and the biggest em. An empty line stands as
    /// tall as the look it would be typed in.
    fn measure(&self, para: &Paragraph<'_>, range: Range<usize>) -> (f32, f32, f32) {
        let mut tallest: Option<(f32, f32, f32)> = None;
        if self.looks.len() > 1 {
            for at in range.clone() {
                let Some(&look) = para.look_of.get(at) else {
                    break;
                };
                let (ascent, descent) = self.metrics[look];
                let em = self.em * self.looks[look].scale;
                tallest = Some(tallest.map_or((ascent, descent, em), |(a, d, e)| {
                    (a.max(ascent), d.max(descent), e.max(em))
                }));
            }
        }
        tallest.unwrap_or_else(|| {
            let look = para
                .look_of
                .get(range.start)
                .or_else(|| para.look_of.get(range.start.wrapping_sub(1)))
                .copied()
                .unwrap_or(0);
            let (ascent, descent) = self.metrics[look];
            (ascent, descent, self.em * self.looks[look].scale)
        })
    }

    /// Shapes `range` of a paragraph as one line, pen starting at (0, 0) on
    /// the baseline, and outlines it when `outline` - a line only being
    /// measured is not.
    ///
    /// The line's runs are laid left to right in the order they are seen,
    /// which for right-to-left text is not the order they are read; each
    /// run is split where its face or its look changes, and shaped piece by
    /// piece, each at its look's size.
    fn set(&self, para: &Paragraph<'_>, range: Range<usize>, outline: bool) -> Line {
        let (ascent, descent, em) = self.measure(para, range.clone());
        if range.is_empty() {
            return Line {
                paths: Vec::new(),
                marks: Vec::new(),
                width: 0.0,
                words: Vec::new(),
                stretches: Vec::new(),
                ascent,
                descent,
                em,
            };
        }
        let look_at = |at: usize| para.look_of.get(at).copied().unwrap_or(0);
        let (levels, runs) = para.bidi.visual_runs(range.clone());
        // One outline per look, made as the look first turns up.
        let mut builders: Vec<Option<PathBuilder>> = self.looks.iter().map(|_| None).collect();
        let mut marks: Vec<Mark> = Vec::new();
        let mut pen = 0.0_f32;
        // Each glyph's cluster, as a byte into the paragraph, the span it
        // advances over, and its look.
        let mut spans: Vec<(usize, f32, f32, usize)> = Vec::new();
        for run in runs {
            let rtl = levels[run.start].is_rtl();
            let mut pieces = Vec::new();
            let mut start = run.start;
            for at in run.clone() {
                if para.face_of[at] != para.face_of[start] || look_at(at) != look_at(start) {
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
                let look = look_at(piece.start);
                let em = self.em * self.looks[look].scale;
                let scale = em / face.units_per_em() as f32;
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
                        let glyph = ttf_parser::GlyphId(info.glyph_id as u16);
                        let x = pen + position.x_offset as f32 * scale;
                        let y = -(position.y_offset as f32 * scale);
                        // A glyph that colours itself - a strike's
                        // picture, layered paints - goes into the line's
                        // marks instead of its outlines: the words' fill
                        // would wash its colours out. Anything else is
                        // an outline in its look's path, as always.
                        let foreground = rgba(self.looks[look].fill);
                        if let Some(mark) = self.mark(face, glyph, x, y, em, foreground) {
                            marks.push(mark);
                        } else {
                            let mut outliner = Outliner {
                                builder: builders[look].get_or_insert_with(PathBuilder::new),
                                scale,
                                x,
                                y,
                            };
                            face.outline_glyph(glyph, &mut outliner);
                        }
                    }
                    let advance = position.x_advance as f32 * scale;
                    spans.push((
                        piece.start + info.cluster as usize,
                        pen,
                        pen + advance,
                        look,
                    ));
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
        let (words, stretches) = if outline {
            let words = words(para.text, range)
                .into_iter()
                .filter_map(|word| {
                    spans
                        .iter()
                        .filter(|(cluster, _, _, _)| word.contains(cluster))
                        .fold(None, |span: Option<(f32, f32)>, &(_, left, right, _)| {
                            Some(span.map_or((left, right), |(l, r)| (l.min(left), r.max(right))))
                        })
                })
                .collect();
            let mut stretches: Vec<(usize, f32, f32)> = Vec::new();
            for &(_, left, right, look) in &spans {
                match stretches.last_mut() {
                    Some((held, _, end)) if *held == look => *end = end.max(right),
                    _ => stretches.push((look, left, right)),
                }
            }
            (words, stretches)
        } else {
            (Vec::new(), Vec::new())
        };
        Line {
            paths: builders
                .into_iter()
                .enumerate()
                .filter_map(|(look, builder)| Some((look, builder?.finish()?)))
                .collect(),
            marks,
            width,
            words,
            stretches,
            ascent,
            descent,
            em,
        }
    }

    /// A glyph's own colours, at its origin `(x, y)` on the baseline and
    /// at `em` pixels: its font's bitmap, decoded and boxed to the strike's
    /// size, or its `COLR` layers outlined in their palette colours, with
    /// `foreground` for the palette's "the text's own colour". `None` for a
    /// plain outline - most glyphs - and for a colour glyph this crate does
    /// not follow, which falls back to its outline as before.
    fn mark(
        &self,
        face: &rustybuzz::Face<'_>,
        glyph: ttf_parser::GlyphId,
        x: f32,
        y: f32,
        em: f32,
        foreground: ttf_parser::RgbaColor,
    ) -> Option<Mark> {
        if let Some(image) = face.glyph_raster_image(glyph, u16::MAX)
            && matches!(
                image.format,
                ttf_parser::RasterImageFormat::PNG
                    | ttf_parser::RasterImageFormat::BitmapPremulBgra32
            )
            && let Some(pixmap) = bitmap(&image)
        {
            // The strike's pixels are in the font's y-up, measured from
            // the glyph's origin to the bitmap's *bottom* edge - both for
            // `CBDT` and for `sbix` - while the canvas goes down. `k`
            // turns a strike pixel into canvas pixels at this size.
            let k = em / f32::from(image.pixels_per_em.max(1));
            let height = f32::from(image.height) * k;
            return Some(Mark::Bitmap {
                x: x + f32::from(image.x) * k,
                y: y - (f32::from(image.y) * k) - height,
                width: f32::from(image.width) * k,
                height,
                pixmap,
            });
        }
        let scale = em / face.units_per_em() as f32;
        let layers = Layering::of(face, glyph, foreground)?;
        let mut placed = Vec::with_capacity(layers.len());
        for (glyph_id, colour) in layers {
            let mut builder = PathBuilder::new();
            face.outline_glyph(
                glyph_id,
                &mut Outliner {
                    builder: &mut builder,
                    scale,
                    x,
                    y,
                },
            );
            // A layer with no outline paints nothing; the rest still do.
            if let Some(path) = builder.finish() {
                placed.push((
                    path,
                    Color::from_rgba8(colour.red, colour.green, colour.blue, colour.alpha),
                ));
            }
        }
        (!placed.is_empty()).then_some(Mark::Layers(placed))
    }
}

/// The strike's bitmap, decoded into pixels the canvas can lay over the
/// words: premultiplied RGBA, whether it arrived as a PNG or as raw
/// `BGRA`. Colour formats only - a monochrome strike is the outline's
/// job, and this crate's caller has an outline to fall back to.
fn bitmap(image: &ttf_parser::RasterGlyphImage<'_>) -> Option<Pixmap> {
    match image.format {
        ttf_parser::RasterImageFormat::PNG => Pixmap::decode_png(image.data).ok(),
        ttf_parser::RasterImageFormat::BitmapPremulBgra32 => {
            let size = tiny_skia::IntSize::from_wh(image.width.into(), image.height.into())?;
            let mut rgba = Vec::with_capacity(image.data.len());
            for pixel in image.data.chunks_exact(4) {
                rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
            // `from_vec` checks the length, so a strike that lies about
            // its size comes out `None` rather than skewed.
            Pixmap::from_vec(rgba, size)
        }
        _ => None,
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
        width: canvas.width(),
        height: canvas.height(),
        overscan: overscan_of(&canvas, width, height),
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
        width: canvas.width(),
        height: canvas.height(),
        overscan: overscan_of(&canvas, width, height),
        block_width: block.0,
        block_height: block.1,
        block_dx: block.2,
        block_dy: block.3,
        words,
    })
}

/// A painted block: (width, height, dx, dy), on [`Rendered`]'s terms.
type Block = (u32, u32, i32, i32);

/// The longest side a canvas may grow to: the largest texture every GPU
/// takes, `concat_core::frame::MAX_SIDE`. Ink past it is cut as all ink
/// past the frame once was, which takes a box several frames wide.
const MAX_SIDE: u32 = 8192;

/// How many times a `width` by `height` frame `canvas` is, along the axis
/// it grew the most; see [`Rendered::overscan`].
fn overscan_of(canvas: &Pixmap, width: u32, height: u32) -> f64 {
    (f64::from(canvas.width()) / f64::from(width.max(1)))
        .max(f64::from(canvas.height()) / f64::from(height.max(1)))
        .max(1.0)
}

/// Where ink lands, as left, top, right and bottom edges in pixels: what
/// the canvas has to hold. Empty until something is added.
#[derive(Clone, Copy)]
struct Ink(Option<[f32; 4]>);

impl Ink {
    /// Takes in `rect`, moved by `(dx, dy)` and grown by `by` on every side.
    fn add(&mut self, rect: Rect, (dx, dy): (f32, f32), by: f32) {
        let edges = [
            rect.left() + dx - by,
            rect.top() + dy - by,
            rect.right() + dx + by,
            rect.bottom() + dy + by,
        ];
        self.0 = Some(match self.0 {
            None => edges,
            Some([l, t, r, b]) => [
                l.min(edges[0]),
                t.min(edges[1]),
                r.max(edges[2]),
                b.max(edges[3]),
            ],
        });
    }

    /// The margin a `width` by `height` canvas needs on each side to hold
    /// the ink, x and y, in whole pixels; nothing for ink inside it. The
    /// same either side, so the canvas's centre does not move, and held so
    /// the canvas grows no larger than [`MAX_SIDE`].
    fn margins(&self, width: u32, height: u32) -> (u32, u32) {
        let Some([left, top, right, bottom]) = self.0 else {
            return (0, 0);
        };
        let past = |low: f32, high: f32, side: u32| -> u32 {
            let over = (-low).max(high - side as f32).max(0.0);
            // A hair over is anti-aliasing's guess at an edge, not ink.
            let over = if over < 0.01 { 0 } else { over.ceil() as u32 };
            over.min(MAX_SIDE.saturating_sub(side) / 2)
        };
        (past(left, right, width), past(top, bottom, height))
    }
}

/// The canvas with the title on it, the block, and every word's box on the
/// canvas in reading order.
fn paint(
    fonts: &Fonts,
    style: &TitleStyle,
    width: u32,
    height: u32,
) -> Result<(Pixmap, Block, Vec<WordRect>), Error> {
    // The frame's canvas, for a title with nothing to paint; one with ink
    // gets a canvas as big as its ink needs, below.
    let empty = Pixmap::new(width, height).ok_or(Error::Canvas(width, height))?;
    let frame_h = height as f32;
    let em = (style.font_size.clamp(0.005, 1.0) as f32) * frame_h;
    let tracking = style.tracking as f32 * frame_h;
    let line_height = style.line_height.max(0.5) as f32;

    // The title's own look and its runs', and which each byte is in.
    let (looks, look_of) = looks_of(style);
    let casting = fonts.cast_looks(style, &looks, &look_of)?;
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
    // A line's metrics are its looks' own faces', whatever they borrow:
    // the pitch and the block should not jump because one character came
    // from elsewhere. A title in one look is the style's face's.
    let metrics: Vec<(f32, f32)> = looks
        .iter()
        .zip(&casting.owns)
        .map(|(look, &face)| {
            let face = &faces[face];
            let size = em * look.scale / face.units_per_em() as f32;
            (
                face.ascender() as f32 * size,
                -(face.descender() as f32) * size,
            )
        })
        .collect();
    let (ascent, descent) = metrics[0];

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
    // once the block's width is known. Each look's colour goes along: a
    // `COLR` palette entry of 0xFFFF means "paint it with the text's own".
    let setter = Setter {
        faces: &faces,
        em,
        tracking,
        looks: &looks,
        metrics: &metrics,
    };
    let lines: Vec<Line> = style
        .content
        .lines()
        .zip(&casting.paragraphs)
        .flat_map(|(text, face_of)| {
            let at = text.as_ptr() as usize - style.content.as_ptr() as usize;
            let para = Paragraph {
                text,
                bidi: ParagraphBidiInfo::new(text, None),
                face_of,
                look_of: look_of.get(at..at + text.len()).unwrap_or(&[]),
            };
            setter.wrap(&para, wrap_w)
        })
        .collect();
    let words_w = lines.iter().map(|line| line.width).fold(0.0, f32::max);
    if words_w <= 0.0 || lines.is_empty() {
        // Nothing to paint: an empty, valid canvas.
        return Ok((empty, (0, 0, 0, 0), Vec::new()));
    }
    // Each baseline below the first by the pitch of the bigger of the two
    // lines it sits between, so a line with a big word in it pushes its
    // neighbours away rather than running into them. In one look every
    // pitch is the same, as it always was.
    let mut drops = Vec::with_capacity(lines.len());
    let mut drop = 0.0_f32;
    for (row, line) in lines.iter().enumerate() {
        if row > 0 {
            drop += line_height * lines[row - 1].em.max(line.em);
        }
        drops.push(drop);
    }
    let first_ascent = lines.first().map_or(ascent, |line| line.ascent);
    let last_descent = lines.last().map_or(descent, |line| line.descent);
    let words_h = drop + first_ascent + last_descent;

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
        // A box wider than the half the anchored edge leaves would run
        // off the canvas and its far end was never drawn: a left-aligned
        // caption came out cut where a centred one did not. It slides back
        // in instead, as far as the canvas allows; one that fits stays on
        // its anchor.
        Align::Left => anchor_x.min(width as f32 - outer_w).max(0.0),
        Align::Center => anchor_x - outer_w / 2.0,
        Align::Right => (anchor_x - outer_w).max(0.0).min(width as f32 - outer_w),
    };
    let left = outer_left + pad_x;
    // The words are centred in a box taller than they are.
    let top = (frame_h - outer_h) / 2.0 + pad_y + (box_h - words_h) / 2.0;
    let block_dx = (outer_left + outer_w / 2.0 - anchor_x).round() as i32;
    let indent_of = |line: &Line| match style.align {
        Align::Left => 0.0,
        Align::Center => (box_w - line.width) / 2.0,
        Align::Right => box_w - line.width,
    };
    let baseline_of = |row: usize| top + first_ascent + drops[row];

    // Each look's words as one path, placed, ruled lines included. Each
    // line is aligned within the box's width and sits on its own
    // baseline. A word's box rides the same indent and baseline, in
    // reading order - a line that paints its glyphs' own colours has no
    // path to place but still has words.
    let mut placed: Vec<PathBuilder> = looks.iter().map(|_| PathBuilder::new()).collect();
    // A look's ruled lines in its own face's metrics, at its own size, so
    // they sit where the face's designer put them; an em-based guess for
    // a face that does not say.
    let rules: Vec<[(f32, f32); 2]> = looks
        .iter()
        .zip(&casting.owns)
        .map(|(look, &face)| {
            let face = &faces[face];
            let em = em * look.scale;
            let upem = face.units_per_em() as f32;
            let metric = |m: Option<ttf_parser::LineMetrics>, fallback: f32| {
                m.map_or((fallback, (em * 0.05).max(1.0)), |m| {
                    (
                        -f32::from(m.position) / upem * em,
                        (f32::from(m.thickness).abs() / upem * em).max(1.0),
                    )
                })
            };
            [
                metric(face.underline_metrics(), em * 0.1),
                metric(face.strikeout_metrics(), -em * 0.3),
            ]
        })
        .collect();
    let mut word_rects = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        let indent = indent_of(line);
        let baseline = baseline_of(row);
        for (look, path) in &line.paths {
            let path = path
                .clone()
                .transform(Transform::from_translate(left + indent, baseline))
                .expect("a translated glyph path stays finite");
            placed[*look].push_path(&path);
        }
        // Rules share the glyphs' fill, outline and shadow.
        for &(look, start_x, end_x) in &line.stretches {
            let wanted = [looks[look].underline, looks[look].strikethrough];
            for (on, (offset, thickness)) in wanted.into_iter().zip(rules[look]) {
                if on
                    && line.width > 0.0
                    && let Some(rect) = Rect::from_xywh(
                        left + indent + start_x,
                        baseline + offset - thickness / 2.0,
                        end_x - start_x,
                        thickness,
                    )
                {
                    placed[look].push_rect(rect);
                }
            }
        }
        let line_top = baseline - line.ascent;
        for &(start_x, end_x) in &line.words {
            word_rects.push(WordRect {
                x: (left + indent + start_x).round() as i32,
                y: line_top.round() as i32,
                width: (end_x - start_x).max(0.0).round() as u32,
                height: (line.ascent + line.descent).round() as u32,
            });
        }
    }
    let words: Vec<(usize, Path)> = placed
        .into_iter()
        .enumerate()
        .filter_map(|(look, builder)| Some((look, builder.finish()?)))
        .collect();

    // Everything about to be painted, in the frame's pixels: the plate,
    // the words with the outer half of their outline, their shadow and
    // its blur, and the glyphs that paint their own colours. The words'
    // advances are not their ink - a letter's outline, an italic's lean, a
    // side bearing below zero all reach past them - so the outlines are
    // measured, not the line. Whatever reaches past the frame's edge
    // grows the canvas by that much on both sides, so it is painted
    // rather than cut, and the canvas's centre stays the frame's.
    let stroke_w = style.stroke_width.max(0.0) as f32 * frame_h;
    let shadow_offset = (em * 0.05, em * 0.07);
    let shadow_blur = (em * 0.04).round().max(1.0) as usize;
    let plate_rect = plate
        .and_then(|_| Rect::from_xywh(outer_left, (frame_h - outer_h) / 2.0, outer_w, outer_h));
    // A pixel more than the edges, for the anti-aliasing either side.
    let mut ink = Ink(None);
    if let Some(rect) = plate_rect {
        ink.add(rect, (0.0, 0.0), 1.0);
    }
    for (look, path) in &words {
        let edge = if stroke_w > 0.0 && looks[*look].edge.is_some() {
            stroke_w
        } else {
            0.0
        };
        ink.add(path.bounds(), (0.0, 0.0), edge + 1.0);
        if style.shadow {
            // The blur is a box run twice, so it spreads twice its radius.
            ink.add(path.bounds(), shadow_offset, 2.0 * shadow_blur as f32 + 1.0);
        }
    }
    for (row, line) in lines.iter().enumerate() {
        let at = (left + indent_of(line), baseline_of(row));
        for mark in &line.marks {
            match mark {
                Mark::Bitmap {
                    x,
                    y,
                    width,
                    height,
                    ..
                } => {
                    if let Some(rect) = Rect::from_xywh(*x, *y, *width, *height) {
                        ink.add(rect, at, 1.0);
                    }
                }
                Mark::Layers(layers) => {
                    for (path, _) in layers {
                        ink.add(path.bounds(), at, 1.0);
                    }
                }
            }
        }
    }
    let (margin_x, margin_y) = ink.margins(width, height);
    let mut canvas = if (margin_x, margin_y) == (0, 0) {
        empty
    } else {
        let (grown_w, grown_h) = (width + 2 * margin_x, height + 2 * margin_y);
        Pixmap::new(grown_w, grown_h).ok_or(Error::Canvas(grown_w, grown_h))?
    };
    // Everything below is laid out in the frame's pixels and painted
    // through this, onto the canvas the frame sits in the middle of.
    let (shift_x, shift_y) = (margin_x as f32, margin_y as f32);
    let shift = Transform::from_translate(shift_x, shift_y);
    for word in &mut word_rects {
        word.x += margin_x as i32;
        word.y += margin_y as i32;
    }

    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };

    // The plate, first and under everything: the whole box.
    if let Some(fill) = plate
        && let Some(rect) = plate_rect
    {
        paint.set_color(fill);
        let radius = (style.background_radius.max(0.0) as f32) * frame_h;
        let mut plate_path = PathBuilder::new();
        push_rounded_rect(&mut plate_path, rect, radius);
        if let Some(plate_path) = plate_path.finish() {
            canvas.fill_path(&plate_path, &paint, FillRule::Winding, shift, None);
        }
    }

    // The shadow: the words again, offset down and right, black, blurred on
    // a layer of their own and laid under the real words - every look's
    // under all of them, one shadow for the title.
    if style.shadow
        && !words.is_empty()
        && let Some(mut layer) = Pixmap::new(canvas.width(), canvas.height())
    {
        let mut shade = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        shade.set_color(Color::from_rgba8(0, 0, 0, 150));
        let offset =
            Transform::from_translate(shadow_offset.0 + shift_x, shadow_offset.1 + shift_y);
        for (_, path) in &words {
            layer.fill_path(path, &shade, FillRule::Winding, offset, None);
        }
        blur(&mut layer, shadow_blur);
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
    // why it is drawn at twice the asked-for width. Every look's outline
    // goes down before any fill, so one word's outline never covers its
    // neighbour's letters.
    if stroke_w > 0.0 {
        let stroke = Stroke {
            width: stroke_w * 2.0,
            line_join: LineJoin::Round,
            line_cap: LineCap::Round,
            ..Stroke::default()
        };
        for (look, path) in &words {
            if let Some(edge) = looks[*look].edge {
                paint.set_color(edge);
                canvas.stroke_path(path, &paint, &stroke, shift, None);
            }
        }
    }

    // The words, each look in its own colour.
    for (look, path) in &words {
        paint.set_color(looks[*look].fill);
        canvas.fill_path(path, &paint, FillRule::Winding, shift, None);
    }

    // The glyphs that paint their own colours - an emoji's picture, a
    // layered face's paints - over the words: their colours are their
    // own, so the fill cannot have them. No shadow and no outline round
    // them either; they are pictures, and a box would be a box.
    // https://github.com/jub0t/Concat/issues/276
    for (row, line) in lines.iter().enumerate() {
        if line.marks.is_empty() {
            continue;
        }
        let indent = indent_of(line);
        let baseline = baseline_of(row);
        let placed = Transform::from_translate(left + indent + shift_x, baseline + shift_y);
        for mark in &line.marks {
            match mark {
                Mark::Bitmap {
                    x,
                    y,
                    width,
                    height,
                    pixmap,
                } => {
                    let scale_x = *width / pixmap.width() as f32;
                    let scale_y = *height / pixmap.height() as f32;
                    canvas.draw_pixmap(
                        0,
                        0,
                        pixmap.as_ref(),
                        &PixmapPaint {
                            // A strike is drawn far bigger than it lands;
                            // nearest would stair-step its round edges.
                            quality: FilterQuality::Bilinear,
                            ..PixmapPaint::default()
                        },
                        Transform::from_row(
                            scale_x,
                            0.0,
                            0.0,
                            scale_y,
                            left + indent + *x + shift_x,
                            baseline + *y + shift_y,
                        ),
                        None,
                    );
                }
                Mark::Layers(layers) => {
                    for (path, layer_colour) in layers {
                        paint.set_color(*layer_colour);
                        canvas.fill_path(path, &paint, FillRule::Winding, placed, None);
                    }
                }
            }
        }
    }

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
        // A shape is painted on the frame alone, as it always was.
        overscan: 1.0,
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
        // A shape is painted on the frame alone, as it always was.
        overscan: 1.0,
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
            underline: false,
            strikethrough: false,
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
            runs: Vec::new(),
        }
    }

    #[test]
    fn decorations_change_pixels_without_moving_the_text() {
        let fonts = Fonts::new();
        let plain = style("First line\nSecond line");
        let base = render_frame(&fonts, &plain, 640, 360).unwrap();
        for (underline, strikethrough) in [(true, false), (false, true), (true, true)] {
            let decorated = TitleStyle {
                underline,
                strikethrough,
                ..plain.clone()
            };
            let painted = render_frame(&fonts, &decorated, 640, 360).unwrap();
            assert_ne!(painted.rgba, base.rgba);
            assert_eq!(painted.words, base.words);
            assert_eq!(painted.block_width, base.block_width);
            assert_eq!(painted.block_height, base.block_height);
            // The PNG export path and live preview share the same painter.
            let exported = render(&fonts, &decorated, 640, 360).unwrap();
            assert_eq!(exported.words, painted.words);
        }
    }

    /// How many of the canvas's pixels inside `word` pass `test`, read
    /// straight from the RGBA, only those mostly opaque.
    fn pixels_in(
        frame: &RenderedFrame,
        word: WordRect,
        test: impl Fn(u8, u8, u8) -> bool,
    ) -> usize {
        let mut count = 0;
        for y in word.y.max(0)..(word.y + word.height as i32).min(frame.height as i32) {
            for x in word.x.max(0)..(word.x + word.width as i32).min(frame.width as i32) {
                let at = (y as usize * frame.width as usize + x as usize) * 4;
                let px = &frame.rgba[at..at + 4];
                if px[3] > 200 && test(px[0], px[1], px[2]) {
                    count += 1;
                }
            }
        }
        count
    }

    fn is_white(r: u8, g: u8, b: u8) -> bool {
        r > 200 && g > 200 && b > 200
    }

    /// "Make" in red, "this" in the title's white: each word is painted in
    /// its own colour, and only its own.
    #[test]
    fn a_coloured_run_paints_its_words_and_only_its_words() {
        let fonts = Fonts::new();
        let mut styled = style("Make this");
        styled.shadow = false;
        styled.runs = vec![TitleRun {
            start: 0,
            end: 4,
            color: Some("#ff0000".into()),
            ..TitleRun::default()
        }];
        let frame = render_frame(&fonts, &styled, 640, 360).expect("renders");
        assert_eq!(frame.words.len(), 2);
        let (make, this) = (frame.words[0], frame.words[1]);
        assert!(pixels_in(&frame, make, is_red) > 50, "the run is red");
        assert_eq!(pixels_in(&frame, make, is_white), 0, "and nothing else");
        assert!(
            pixels_in(&frame, this, is_white) > 50,
            "its neighbour is white"
        );
        assert_eq!(pixels_in(&frame, this, is_red), 0, "and not red");
        // A colour moves no word: the boxes are the title's in one look.
        styled.runs.clear();
        let plain = render_frame(&fonts, &styled, 640, 360).expect("renders");
        assert_eq!(plain.words, frame.words);
        assert_eq!(
            (plain.block_width, plain.block_height),
            (frame.block_width, frame.block_height)
        );
    }

    /// A word twice the size is twice as wide and makes its line taller:
    /// the block grows to hold it and the next line moves down.
    #[test]
    fn a_bigger_run_is_bigger_and_its_line_is_taller() {
        let fonts = Fonts::new();
        let plain = style("big word\nnext");
        let mut styled = plain.clone();
        styled.runs = vec![TitleRun {
            start: 0,
            end: 3,
            font_scale: Some(2.0),
            ..TitleRun::default()
        }];
        let before = render_frame(&fonts, &plain, 640, 360).expect("renders");
        let after = render_frame(&fonts, &styled, 640, 360).expect("renders");
        let ratio = after.words[0].width as f32 / before.words[0].width as f32;
        assert!((1.8..2.2).contains(&ratio), "twice as wide: {ratio}");
        assert!(
            after.words[0].height > before.words[0].height,
            "its line stands taller"
        );
        assert_eq!(after.words[1].width, before.words[1].width, "word is not");
        assert!(after.block_height > before.block_height);
        let gap = |frame: &RenderedFrame| frame.words[2].y - frame.words[0].y;
        assert!(gap(&after) > gap(&before), "the next line moved down");
    }

    /// An underline on one word is ruled under that word alone, and a
    /// bold run is set in the heavier face: wider, with more ink.
    #[test]
    fn a_runs_rule_and_weight_stay_on_its_words() {
        let fonts = Fonts::new();
        let mut plain = style("ruled plain");
        plain.font_family = BUNDLED_FAMILY.to_owned();
        plain.font_weight = 400.0;
        plain.shadow = false;
        let mut styled = plain.clone();
        styled.runs = vec![TitleRun {
            start: 0,
            end: 5,
            underline: Some(true),
            ..TitleRun::default()
        }];
        // Full HD: at 360 lines a rule is a pixel thick and never solid.
        let before = render_frame(&fonts, &plain, 1920, 1080).expect("renders");
        let after = render_frame(&fonts, &styled, 1920, 1080).expect("renders");
        let ink =
            |frame: &RenderedFrame, word: usize| pixels_in(frame, frame.words[word], is_white);
        assert!(ink(&after, 0) > ink(&before, 0), "the run gained its rule");
        assert_eq!(ink(&after, 1), ink(&before, 1), "its neighbour did not");

        styled.runs = vec![TitleRun {
            start: 0,
            end: 5,
            font_weight: Some(800.0),
            ..TitleRun::default()
        }];
        let bold = render_frame(&fonts, &styled, 1920, 1080).expect("renders");
        assert!(ink(&bold, 0) > ink(&before, 0), "a heavier face, more ink");
        assert!(bold.words[0].width > before.words[0].width, "and wider");
        // Its neighbour is the same word, moved along by however much
        // wider the bold one is: only its edges land on other pixels.
        assert_eq!(bold.words[1].width, before.words[1].width);
        let (moved, still) = (ink(&bold, 1) as f32, ink(&before, 1) as f32);
        assert!(
            (moved / still - 1.0).abs() < 0.03,
            "{moved} against {still}"
        );
    }

    /// Runs count characters, not bytes: a run over "été" covers those
    /// three letters, accents and all.
    #[test]
    fn a_run_counts_characters() {
        let fonts = Fonts::new();
        let mut styled = style("été ici");
        styled.shadow = false;
        styled.runs = vec![TitleRun {
            start: 0,
            end: 3,
            color: Some("#ff0000".into()),
            ..TitleRun::default()
        }];
        let frame = render_frame(&fonts, &styled, 640, 360).expect("renders");
        assert!(pixels_in(&frame, frame.words[0], is_red) > 30);
        assert_eq!(pixels_in(&frame, frame.words[0], is_white), 0);
        assert_eq!(pixels_in(&frame, frame.words[1], is_red), 0);
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

    /// A caption in any language the app speaks draws its words with the
    /// bundled faces alone, as on a phone whose fonts lend nothing: every
    /// character lands on a face that has a glyph for it, never on a box.
    #[test]
    fn every_language_the_app_speaks_draws_without_the_machines_fonts() {
        let fonts = Fonts::with_system(false);
        let lines = [
            ("Thai", "สวัสดีครับ ยินดีต้อนรับสู่คำบรรยาย"),
            ("Arabic", "مرحبا بك في الترجمة"),
            ("Persian", "سلام، به زیرنویس خوش آمدید"),
            ("Urdu", "خوش آمدید، یہ سب ٹائٹل ہے"),
            ("Hebrew", "שלום, ברוכים הבאים לכתוביות"),
            ("Hindi", "नमस्ते, उपशीर्षक में आपका स्वागत है"),
            ("Bengali", "হ্যালো, সাবটাইটেলে স্বাগতম"),
            ("Korean", "안녕하세요, 자막에 오신 것을 환영합니다"),
            ("Simplified Chinese", "你好，欢迎使用字幕"),
            ("Traditional Chinese", "你好，歡迎使用字幕"),
            ("Japanese", "こんにちは、字幕へようこそ。カタカナ"),
            ("Russian", "Привет, добро пожаловать в субтитры"),
            ("Ukrainian", "Привіт, ласкаво просимо до субтитрів ґ є ї"),
            ("Vietnamese", "Xin chào, chào mừng đến với phụ đề ữ ỵ ẫ"),
            ("Turkish", "Merhaba, altyazılara hoş geldiniz İ ğ ş"),
            ("Polish", "Cześć, witaj w napisach ą ę ł ż ź"),
            ("Croatian", "Bok, dobrodošli u titlove č ć đ š ž"),
            ("German", "Grüß dich, willkommen bei den Untertiteln ß"),
            ("Greek", "Γεια σας, καλώς ήρθατε στους υπότιτλους"),
        ];
        let mut boxes = Vec::new();
        for (language, line) in lines {
            let casting = fonts.cast(&style(line)).expect("casts");
            let faces: Vec<_> = casting
                .faces
                .iter()
                .map(|(data, index)| {
                    ttf_parser::Face::parse((**data).as_ref(), *index).expect("parses")
                })
                .collect();
            for (at, ch) in line.char_indices() {
                if ch.is_whitespace() || joins(ch) {
                    continue;
                }
                let face = &faces[casting.paragraphs[0][at]];
                if face.glyph_index(ch).is_none() {
                    boxes.push(format!("{language}: {ch:?} (U+{:04X})", ch as u32));
                }
            }
        }
        assert!(boxes.is_empty(), "drawn as boxes:\n{}", boxes.join("\n"));
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

    /// A sized box wider than half the canvas, aligned to an edge, would
    /// start at the anchor and run off the canvas: its words were cut where
    /// a centred one's were not. It slides back in whole.
    #[test]
    fn a_wide_box_aligned_to_an_edge_stays_on_the_canvas() {
        let fonts = Fonts::new();
        let (width, height) = (640, 360);
        for align in [Align::Left, Align::Right] {
            let mut wide = style("Escorpio - Casa 6 and Capricornio Cazimi");
            wide.align = align;
            wide.shadow = false;
            wide.max_width = 0.8;
            let out = render(&fonts, &wide, width, height).expect("renders");
            assert!(!out.words.is_empty(), "{align:?}");
            for word in &out.words {
                assert!(
                    word.x >= 0 && word.x as u32 + word.width <= width,
                    "{align:?}: a word at {}..{} leaves the {width}-wide canvas",
                    word.x,
                    word.x as u32 + word.width
                );
            }
            let half = out.block_width as i32 / 2;
            let centre = width as i32 / 2 + out.block_dx;
            assert!(
                centre - half >= -1 && centre + half <= width as i32 + 1,
                "{align:?}"
            );
        }
    }

    /// Whether any paint touches the canvas's outermost column or row: ink
    /// that reached the edge, and so was cut there.
    fn touches_an_edge(png: &[u8]) -> bool {
        let pixmap = Pixmap::decode_png(png).expect("our own PNG decodes");
        let (width, height) = (pixmap.width(), pixmap.height());
        pixmap.pixels().iter().enumerate().any(|(index, pixel)| {
            let (x, y) = (index as u32 % width, index as u32 / width);
            pixel.alpha() > 0 && (x == 0 || y == 0 || x == width - 1 || y == height - 1)
        })
    }

    /// A title inside the frame is painted on the frame, as every title
    /// was: nothing about it changes for the canvas being able to grow.
    #[test]
    fn a_title_inside_the_frame_is_painted_on_the_frame() {
        let fonts = Fonts::new();
        let mut outlined = style("Your text");
        outlined.stroke_width = 0.01;
        outlined.background = "#000000".to_owned();
        let out = render(&fonts, &outlined, 640, 360).expect("renders");
        assert_eq!((out.width, out.height), (640, 360));
        assert_eq!(out.overscan, 1.0);
        assert!(!touches_an_edge(&out.png));
    }

    /// The situations that cut a title's sides off - its box pulled wider
    /// than the frame with the side grips, a word too long for any line, a
    /// line as wide as the frame with an outline, a shadow and an italic's
    /// lean past its last letter - all paint whole: the canvas grows by
    /// the same margin either side, so nothing reaches its edge and its
    /// centre is still the frame's. Scaled down, moved or turned, such a
    /// title showed the cut as a straight edge through its plate or its
    /// letters.
    #[test]
    fn ink_past_the_frame_grows_the_canvas_rather_than_being_cut() {
        let fonts = Fonts::new();
        let (width, height) = (640_u32, 360_u32);
        let mut boxed = style("a box wider than the frame");
        boxed.background = "#2040a0".to_owned();
        boxed.max_width = 1.6;
        let mut long = style("Unbelievably");
        long.font_family = BUNDLED_FAMILY.to_owned();
        long.font_size = 0.4;
        long.stroke_width = 0.02;
        // A word sized so its letters' advances fill the frame but for two
        // pixels a side: its outline, shadow and lean run past the edge.
        let mut edge = style("Overhang");
        edge.font_family = BUNDLED_FAMILY.to_owned();
        edge.italic = true;
        edge.shadow = false;
        let natural = render(&fonts, &edge, width, height).expect("renders");
        edge.font_size *= f64::from(width - 4) / f64::from(natural.block_width);
        let fitted = render(&fonts, &edge, width, height).expect("renders");
        assert!(
            fitted.block_width <= width && fitted.block_width >= width - 8,
            "{}",
            fitted.block_width
        );
        edge.shadow = true;
        edge.stroke_width = 0.02;
        let mut left = boxed.clone();
        left.align = Align::Left;
        for (name, title) in [
            ("box", boxed),
            ("word", long),
            ("edge", edge),
            ("left", left),
        ] {
            let out = render(&fonts, &title, width, height).expect("renders");
            assert!(out.width > width, "{name}: {} wide", out.width);
            assert_eq!(
                (out.width - width) % 2,
                0,
                "{name}: the same margin either side"
            );
            assert_eq!((out.height - height) % 2, 0, "{name}");
            let expected = (f64::from(out.width) / f64::from(width))
                .max(f64::from(out.height) / f64::from(height));
            assert_eq!(out.overscan, expected, "{name}");
            assert!(!touches_an_edge(&out.png), "{name}: ink was cut");
            for word in &out.words {
                assert!(
                    word.x >= 0 && word.x as u32 + word.width <= out.width,
                    "{name}: a word at {} leaves the canvas",
                    word.x
                );
            }
            // The live painting is the same canvas, pixel for pixel.
            let live = render_frame(&fonts, &title, width, height).expect("renders");
            assert_eq!((live.width, live.height), (out.width, out.height), "{name}");
            assert_eq!(live.words, out.words, "{name}");
        }
    }

    /// A box wider than the frame keeps its plate whole and centred on
    /// the canvas: the canvas's centre is still where the clip's is.
    #[test]
    fn a_box_wider_than_the_frame_is_centred_on_the_grown_canvas() {
        let fonts = Fonts::new();
        let mut boxed = style("wide");
        boxed.background = "#2040a0".to_owned();
        boxed.shadow = false;
        boxed.max_width = 1.5;
        let out = render(&fonts, &boxed, 640, 360).expect("renders");
        assert_eq!(out.block_width, 960);
        assert_eq!(out.block_dx, 0);
        let (left, right) = painted_span(&out.png);
        let middle = (left + right) as f32 / 2.0;
        assert!(
            (middle - out.width as f32 / 2.0).abs() <= 1.0,
            "{left}..{right} in {}",
            out.width
        );
        assert!(right - left + 1 >= 959, "the plate whole: {left}..{right}");
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
        assert!(draws(&lent, '漢').is_some() && draws(&lent, '字').is_some());
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

    // ── colour emoji: https://github.com/jub0t/Concat/issues/276 ──

    /// The solid pixels whose colour passes: the words are white and
    /// their shadow and outline grey, so anything that passes is a
    /// picture's own.
    fn coloured_pixels(png: &[u8], passes: impl Fn(u8, u8, u8) -> bool) -> usize {
        let pixmap = Pixmap::decode_png(png).expect("our own PNG decodes");
        pixmap
            .pixels()
            .iter()
            .filter(|p| p.alpha() > 200 && passes(p.red(), p.green(), p.blue()))
            .count()
    }

    fn is_red(r: u8, g: u8, b: u8) -> bool {
        r > 150 && g < 90 && b < 90
    }

    fn is_blue(r: u8, g: u8, b: u8) -> bool {
        b > 150 && r < 90 && g < 90
    }

    /// The font's bytes with `extra` tables in the directory: laid out
    /// four bytes at a time as a sfnt likes, the directory in tag order,
    /// checksums summed. Enough for a test to give a real face tables it
    /// does not carry.
    fn with_tables(base: &[u8], extra: &[(&[u8], &[u8])]) -> Vec<u8> {
        let tables = usize::from(u16::from_be_bytes([base[4], base[5]]));
        let mut entries: Vec<(Vec<u8>, &[u8])> = Vec::with_capacity(tables + extra.len());
        for index in 0..tables {
            let at = 12 + index * 16;
            let word = |at: usize| {
                u32::from_be_bytes(base[at..at + 4].try_into().expect("four bytes")) as usize
            };
            let (offset, length) = (word(at + 8), word(at + 12));
            entries.push((base[at..at + 4].to_vec(), &base[offset..offset + length]));
        }
        entries.extend(extra.iter().map(|(tag, data)| (tag.to_vec(), *data)));
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let count = entries.len() as u16;
        let entry_selector = (u16::BITS - 1 - count.leading_zeros()) as u16;
        let search_range = 16u16 << entry_selector;
        let mut out = Vec::new();
        out.extend_from_slice(&base[..4]); // the sfnt version
        out.extend_from_slice(&count.to_be_bytes());
        out.extend_from_slice(&search_range.to_be_bytes());
        out.extend_from_slice(&entry_selector.to_be_bytes());
        out.extend_from_slice(&(count * 16 - search_range).to_be_bytes());

        // Each table padded to four bytes, as a sfnt likes.
        let padded: Vec<usize> = entries.iter().map(|(_, d)| (d.len() + 3) & !3).collect();
        let mut offsets = Vec::with_capacity(entries.len());
        let mut next = 12 + entries.len() * 16;
        for len in &padded {
            offsets.push(next);
            next += len;
        }
        for ((tag, data), offset) in entries.iter().zip(&offsets) {
            let mut sum = 0u32;
            for word in data.chunks(4) {
                let mut bytes = [0u8; 4];
                bytes[..word.len()].copy_from_slice(word);
                sum = sum.wrapping_add(u32::from_be_bytes(bytes));
            }
            out.extend_from_slice(tag);
            out.extend_from_slice(&sum.to_be_bytes());
            out.extend_from_slice(&(*offset as u32).to_be_bytes());
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        }
        for ((_, data), len) in entries.iter().zip(&padded) {
            out.extend_from_slice(data);
            out.resize(out.len() + (len - data.len()), 0);
        }
        out
    }

    /// The bundled face with `COLR` and `CPAL` laid over it: the glyph
    /// for `A` painted as two layered outlines - `L` in the palette's
    /// red, `I` in its blue. A colour face in the bytes a colour face
    /// ships, cut from a font the test already carries, so it runs on a
    /// machine with no fonts installed at all.
    fn layered_face() -> Vec<u8> {
        let face = ttf_parser::Face::parse(BUNDLED[0], 0).expect("the bundled face parses");
        let base = face.glyph_index('A').expect("an A").0;
        let first = face.glyph_index('L').expect("an L").0;
        let second = face.glyph_index('I').expect("an I").0;

        // COLR v0: a 14-byte header, one base glyph, two layers.
        let mut colr = Vec::new();
        colr.extend_from_slice(&0u16.to_be_bytes()); // version
        colr.extend_from_slice(&1u16.to_be_bytes()); // numBaseGlyphRecords
        colr.extend_from_slice(&14u32.to_be_bytes()); // baseGlyphRecordsOffset
        colr.extend_from_slice(&20u32.to_be_bytes()); // layerRecordsOffset
        colr.extend_from_slice(&2u16.to_be_bytes()); // numLayerRecords
        colr.extend_from_slice(&base.to_be_bytes());
        colr.extend_from_slice(&0u16.to_be_bytes()); // firstLayerIndex
        colr.extend_from_slice(&2u16.to_be_bytes()); // numLayers
        colr.extend_from_slice(&first.to_be_bytes());
        colr.extend_from_slice(&0u16.to_be_bytes()); // palette entry: red
        colr.extend_from_slice(&second.to_be_bytes());
        colr.extend_from_slice(&1u16.to_be_bytes()); // palette entry: blue

        // CPAL v0: one palette of two entries, records at byte 14 - just
        // after the header's 12 bytes and the one palette index.
        let mut cpal = Vec::new();
        cpal.extend_from_slice(&0u16.to_be_bytes()); // version
        cpal.extend_from_slice(&2u16.to_be_bytes()); // numPaletteEntries
        cpal.extend_from_slice(&1u16.to_be_bytes()); // numPalettes
        cpal.extend_from_slice(&2u16.to_be_bytes()); // numColorRecords
        cpal.extend_from_slice(&14u32.to_be_bytes()); // colorRecordsArrayOffset
        cpal.extend_from_slice(&0u16.to_be_bytes()); // colorRecordIndices[0]
        cpal.extend_from_slice(&[0, 0, 255, 255]); // red: blue, green, red, alpha
        cpal.extend_from_slice(&[255, 0, 0, 255]); // blue

        with_tables(BUNDLED[0], &[(&b"COLR"[..], &colr), (&b"CPAL"[..], &cpal)])
    }

    /// A font set with no system fonts at all: the layered face and the
    /// bundled weights around it, no more.
    fn layered_fonts() -> Fonts {
        let mut db = fontdb::Database::new();
        db.load_font_data(layered_face());
        for face in &BUNDLED[1..] {
            db.load_font_data(face.to_vec());
        }
        // None of the weights around it is a regular, so a regular in
        // the database is the layered face - if it parsed at all.
        assert!(
            db.faces()
                .any(|info| { info.weight.0 == 400 && info.style == fontdb::Style::Normal }),
            "the layered face parses"
        );
        Fonts {
            db,
            cache: Mutex::default(),
        }
    }

    /// A face that paints a glyph in colours of its own draws it, and
    /// those colours are what lands: layered `COLR` paints over the
    /// words, in their palette's red and blue rather than the fill's
    /// white - which is all the same face without those tables can
    /// paint. And a colour face is the one borrowed first, whatever
    /// weight the title asked for.
    #[test]
    fn a_colr_face_paints_its_layers_in_their_own_colours() {
        let fonts = layered_fonts();
        let mut title = style("A");
        title.font_family = BUNDLED_FAMILY.to_owned();
        title.font_weight = 400.0;
        title.shadow = false;
        // Big enough that each layer's pixels are counted past doubt.
        title.font_size = 0.2;
        let layered = render(&fonts, &title, 640, 360).expect("renders");
        assert!(
            coloured_pixels(&layered.png, is_red) > 20,
            "the first layer's red"
        );
        assert!(
            coloured_pixels(&layered.png, is_blue) > 20,
            "the second layer's blue"
        );

        // The same face without those two tables paints only what the
        // words' fill says: no red and no blue anywhere.
        let mut plain_db = fontdb::Database::new();
        plain_db.load_font_data(BUNDLED[0].to_vec());
        let plain_fonts = Fonts {
            db: plain_db,
            cache: Mutex::default(),
        };
        let plain = render(&plain_fonts, &title, 640, 360).expect("renders");
        assert_eq!(coloured_pixels(&plain.png, is_red), 0);
        assert_eq!(coloured_pixels(&plain.png, is_blue), 0);

        // And the faces say so: `draws` calls the layered one colour
        // and the plain one an outline, and nearest takes the colour
        // face before weight ever comes into it.
        let bytes = layered_face();
        let cut = ttf_parser::Face::parse(&bytes, 0).expect("parses");
        assert_eq!(draws(&cut, 'A'), Some(Draw::Colour));
        let bundled = ttf_parser::Face::parse(BUNDLED[0], 0).expect("parses");
        assert_eq!(draws(&bundled, 'A'), Some(Draw::Outline));
        assert_eq!(draws(&bundled, '漢'), None, "no glyph, no drawing");

        let want = Want {
            weight: 700,
            italic: false,
        };
        let id = |weight: u16, slant: fontdb::Style| {
            fonts
                .db
                .faces()
                .find(|info| info.weight.0 == weight && info.style == slant)
                .expect("the face")
                .id
        };
        let bold = id(700, fontdb::Style::Normal);
        let colour_face = id(400, fontdb::Style::Normal);
        let mut covers = Covers::default();
        covers.plain.push(bold);
        covers.colour.push(colour_face);
        assert_eq!(
            fonts.nearest(Some(&covers), want),
            Some(colour_face),
            "colour before weight"
        );
        covers.colour.clear();
        assert_eq!(
            fonts.nearest(Some(&covers), want),
            Some(bold),
            "weight among the plain"
        );
        assert_eq!(fonts.nearest(None, want), None);
    }

    /// An emoji the style's face lacks is lent to a face that draws it
    /// in colour, and painted in those colours rather than the words'
    /// own. Skipped on a machine with no colour emoji face.
    #[test]
    fn an_emoji_is_painted_in_the_colours_of_the_face_that_draws_it() {
        let fonts = Fonts::new();
        let mut party = style("party 🎉 ❤️");
        party.font_family = BUNDLED_FAMILY.to_owned();
        party.shadow = false;
        let casting = fonts.cast(&party).expect("casts");
        let at = party.content.find('🎉').expect("the emoji");
        let lent = casting
            .faces
            .get(casting.paragraphs[0][at])
            .and_then(|(data, index)| ttf_parser::Face::parse((**data).as_ref(), *index).ok())
            .is_some_and(|face| draws(&face, '🎉') == Some(Draw::Colour));
        if !lent {
            eprintln!("no colour face on this machine draws 🎉; skipped");
            return;
        }
        let out = render(&fonts, &party, 640, 360).expect("renders");
        let picture = coloured_pixels(&out.png, |r, g, b| {
            r.max(g).max(b).saturating_sub(r.min(g).min(b)) > 60
        });
        assert!(
            picture > 50,
            "{picture} coloured pixels: the emoji came out in colour"
        );
    }
}

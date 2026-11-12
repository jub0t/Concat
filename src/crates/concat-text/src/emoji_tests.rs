// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How colour emoji land on a title: in their own colours, sized to the
//! em, on the baseline, laid over the words, and left alone by the
//! title's fill, outline and shadow.
//!
//! Most cases install small colour faces assembled here from a font this
//! crate bundles, so they draw the same on a machine with no fonts of its
//! own. The cases that need real emoji sequences borrow the machine's
//! colour emoji face, and return early with a note where it has none.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{
    Align, BUNDLED_FAMILY, Fonts, NOTO_SANS_THAI, RenderedFrame, TitleRun, TitleStyle, render_frame,
};

/// The frame every case is painted on.
const WIDE: u32 = 640;
const TALL: u32 = 400;
/// A font size that makes an 80-pixel em on that frame.
const SIZE: f64 = 0.2;
const EM: f32 = 80.0;

/// The characters the assembled faces give pictures to.
const GRIN: char = '\u{1F600}';
const WINK: char = '\u{1F609}';

/// A title in the bundled face, white, with nothing around its words.
fn title(content: &str) -> TitleStyle {
    TitleStyle {
        content: content.to_owned(),
        font_family: BUNDLED_FAMILY.to_owned(),
        font_size: SIZE,
        font_weight: 400.0,
        italic: false,
        underline: false,
        strikethrough: false,
        color: "#ffffff".to_owned(),
        align: Align::Center,
        stroke_width: 0.0,
        stroke_color: "#000000".to_owned(),
        shadow: false,
        background: String::new(),
        background_radius: 0.0,
        background_padding_x: 0.0,
        background_padding_y: 0.0,
        line_height: 1.2,
        tracking: 0.0,
        max_width: 0.0,
        max_height: 0.0,
        runs: Vec::new(),
    }
}

// ── pixels ──

type Rgba = [u8; 4];

fn solid(p: Rgba) -> bool {
    p[3] > 200
}
fn is_green(p: Rgba) -> bool {
    solid(p) && p[1] > 180 && p[0] < 80 && p[2] < 80
}
fn is_red(p: Rgba) -> bool {
    solid(p) && p[0] > 180 && p[1] < 80 && p[2] < 80
}
fn is_blue(p: Rgba) -> bool {
    solid(p) && p[2] > 180 && p[0] < 80 && p[1] < 80
}
fn is_magenta(p: Rgba) -> bool {
    solid(p) && p[0] > 180 && p[2] > 180 && p[1] < 80
}
fn is_white(p: Rgba) -> bool {
    solid(p) && p[0] > 200 && p[1] > 200 && p[2] > 200
}
/// Strongly coloured: what no white, grey or black word paints.
fn is_saturated(p: Rgba) -> bool {
    let (high, low) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
    solid(p) && high - low > 60
}

fn pixels(frame: &RenderedFrame) -> impl Iterator<Item = (u32, u32, Rgba)> + '_ {
    frame.rgba.chunks_exact(4).enumerate().map(|(at, p)| {
        let at = at as u32;
        (at % frame.width, at / frame.width, [p[0], p[1], p[2], p[3]])
    })
}

fn count(frame: &RenderedFrame, wanted: fn(Rgba) -> bool) -> usize {
    pixels(frame).filter(|&(_, _, p)| wanted(p)).count()
}

/// Left, top, right and bottom (both exclusive) of the pixels that pass.
fn extent(frame: &RenderedFrame, wanted: fn(Rgba) -> bool) -> Option<[i64; 4]> {
    pixels(frame)
        .filter(|&(_, _, p)| wanted(p))
        .fold(None, |held, (x, y, _)| {
            let (x, y) = (i64::from(x), i64::from(y));
            Some(match held {
                None => [x, y, x + 1, y + 1],
                Some([l, t, r, b]) => [l.min(x), t.min(y), r.max(x + 1), b.max(y + 1)],
            })
        })
}

fn width_of(span: [i64; 4]) -> f32 {
    (span[2] - span[0]) as f32
}

fn height_of(span: [i64; 4]) -> f32 {
    (span[3] - span[1]) as f32
}

fn near(got: f32, want: f32, slack: f32) -> bool {
    (got - want).abs() <= slack
}

fn paint(fonts: &Fonts, style: &TitleStyle) -> RenderedFrame {
    render_frame(fonts, style, WIDE, TALL).expect("the title renders")
}

// ── colour faces, assembled ──

fn push16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn push32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// A font's tables by tag.
fn tables_of(font: &[u8]) -> BTreeMap<[u8; 4], Vec<u8>> {
    let word = |at: usize| u32::from_be_bytes(font[at..at + 4].try_into().expect("4 bytes"));
    let count = usize::from(u16::from_be_bytes([font[4], font[5]]));
    (0..count)
        .map(|index| {
            let record = 12 + 16 * index;
            let tag: [u8; 4] = font[record..record + 4].try_into().expect("a tag");
            let (offset, length) = (word(record + 8) as usize, word(record + 12) as usize);
            (tag, font[offset..offset + length].to_vec())
        })
        .collect()
}

/// A TrueType file holding `tables`, in tag order, each on a four-byte
/// boundary.
fn font_of(tables: &BTreeMap<[u8; 4], Vec<u8>>) -> Vec<u8> {
    let count = tables.len();
    let log = usize::BITS - 1 - count.leading_zeros();
    let range = 16 * (1usize << log);
    let mut out = Vec::new();
    push32(&mut out, 0x0001_0000);
    push16(&mut out, count as u16);
    push16(&mut out, range as u16);
    push16(&mut out, log as u16);
    push16(&mut out, (count * 16 - range) as u16);
    let mut offset = 12 + 16 * count;
    for (tag, data) in tables {
        let sum = data.chunks(4).fold(0u32, |sum, piece| {
            let mut word = [0u8; 4];
            word[..piece.len()].copy_from_slice(piece);
            sum.wrapping_add(u32::from_be_bytes(word))
        });
        out.extend_from_slice(tag);
        push32(&mut out, sum);
        push32(&mut out, offset as u32);
        push32(&mut out, data.len() as u32);
        offset += data.len().next_multiple_of(4);
    }
    for data in tables.values() {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

/// The glyph the bundled Thai face draws `letter` with: the outlines the
/// assembled faces borrow.
fn glyph(letter: char) -> u16 {
    let face = ttf_parser::Face::parse(NOTO_SANS_THAI, 0).expect("the Thai face parses");
    face.glyph_index(letter).expect("the Thai face has it").0
}

fn glyph_count() -> u16 {
    ttf_parser::Face::parse(NOTO_SANS_THAI, 0)
        .expect("the Thai face parses")
        .number_of_glyphs()
}

/// A square PNG of one colour.
fn png(side: u32, colour: Rgba) -> Vec<u8> {
    let mut pixmap = tiny_skia::Pixmap::new(side, side).expect("a pixmap");
    pixmap.fill(tiny_skia::Color::from_rgba8(
        colour[0], colour[1], colour[2], colour[3],
    ));
    pixmap.encode_png().expect("encodes")
}

/// A PNG whose header names its size but whose pixels are noise: a
/// strike that cannot be decoded.
fn broken_png(side: u32) -> Vec<u8> {
    let mut bytes = png(side, [0, 255, 0, 255]);
    // Signature and header chunk kept, everything after them garbled.
    bytes.truncate(33);
    bytes.extend_from_slice(&[0x5A; 64]);
    bytes
}

/// One picture in a bitmap strike: the letter whose glyph it stands
/// for, its origin offsets in strike pixels, and its PNG.
struct Picture {
    letter: char,
    x: i16,
    y: i16,
    png: Vec<u8>,
}

/// A colour face to assemble from the bundled Thai face. Each emoji is
/// mapped to the glyph of a Thai letter; that glyph keeps its outline,
/// and may be given layers, pictures in strikes, or both.
#[derive(Default)]
struct Recipe {
    /// Emoji and the letter whose glyph draws it. Nothing else is mapped.
    map: Vec<(char, char)>,
    /// Layered glyphs: the base letter and its layers, bottom first, as
    /// letter and palette entry (0xFFFF for the text's own colour).
    layers: Vec<(char, Vec<(char, u16)>)>,
    /// The palette, RGBA.
    palette: Vec<Rgba>,
    /// Bitmap strikes: pixels per em and their pictures.
    strikes: Vec<(u16, Vec<Picture>)>,
    /// The face's weight, where it should not be the base face's.
    weight: Option<u16>,
    /// Sequences the face joins into one glyph: the characters, as the
    /// letters whose glyphs they map to, and the letter whose glyph is the
    /// joined picture. Each character must be in `map`.
    joins: Vec<(Vec<char>, char)>,
}

impl Recipe {
    fn build(&self) -> Vec<u8> {
        let mut tables = tables_of(NOTO_SANS_THAI);
        tables.insert(*b"cmap", self.cmap());
        if !self.layers.is_empty() {
            tables.insert(*b"COLR", self.colr());
            tables.insert(*b"CPAL", self.cpal());
        }
        if !self.strikes.is_empty() {
            tables.insert(*b"sbix", self.sbix());
        }
        if !self.joins.is_empty() {
            tables.insert(*b"GSUB", self.gsub());
        }
        if let Some(weight) = self.weight {
            let os2 = tables.get_mut(b"OS/2").expect("an OS/2 table");
            os2[4..6].copy_from_slice(&weight.to_be_bytes());
        }
        font_of(&tables)
    }

    /// A format 12 map of the emoji alone.
    fn cmap(&self) -> Vec<u8> {
        let mut groups: Vec<(u32, u16)> = self
            .map
            .iter()
            .map(|&(emoji, letter)| (emoji as u32, glyph(letter)))
            .collect();
        groups.sort_unstable();
        let mut out = Vec::new();
        push16(&mut out, 0);
        push16(&mut out, 1);
        push16(&mut out, 3); // Windows
        push16(&mut out, 10); // full Unicode
        push32(&mut out, 12);
        push16(&mut out, 12); // format
        push16(&mut out, 0);
        push32(&mut out, 16 + 12 * groups.len() as u32);
        push32(&mut out, 0);
        push32(&mut out, groups.len() as u32);
        for (code, id) in groups {
            push32(&mut out, code);
            push32(&mut out, code);
            push32(&mut out, u32::from(id));
        }
        out
    }

    /// One `liga` lookup that joins each sequence into its glyph, the way
    /// an emoji face joins a family, a flag or a skin tone.
    fn gsub(&self) -> Vec<u8> {
        // Ligatures grouped under their first glyph, in glyph order.
        let mut sets: BTreeMap<u16, Vec<(Vec<u16>, u16)>> = BTreeMap::new();
        for (letters, joined) in &self.joins {
            let ids: Vec<u16> = letters.iter().map(|&letter| glyph(letter)).collect();
            sets.entry(ids[0])
                .or_default()
                .push((ids[1..].to_vec(), glyph(*joined)));
        }
        // The substitution subtable: format, coverage, then the sets.
        let mut subtable = Vec::new();
        let head = 6 + 2 * sets.len();
        let mut coverage = Vec::new();
        push16(&mut coverage, 1);
        push16(&mut coverage, sets.len() as u16);
        for first in sets.keys() {
            push16(&mut coverage, *first);
        }
        push16(&mut subtable, 1);
        push16(&mut subtable, head as u16);
        push16(&mut subtable, sets.len() as u16);
        let mut bodies = Vec::new();
        let mut offset = head + coverage.len();
        for ligatures in sets.values() {
            push16(&mut subtable, offset as u16);
            let mut set = Vec::new();
            push16(&mut set, ligatures.len() as u16);
            let mut ligs = Vec::new();
            let mut at = 2 + 2 * ligatures.len();
            for (rest, joined) in ligatures {
                push16(&mut set, at as u16);
                push16(&mut ligs, *joined);
                push16(&mut ligs, rest.len() as u16 + 1);
                for id in rest {
                    push16(&mut ligs, *id);
                }
                at = 2 + 2 * ligatures.len() + ligs.len();
            }
            set.extend_from_slice(&ligs);
            offset += set.len();
            bodies.push(set);
        }
        subtable.extend_from_slice(&coverage);
        for body in bodies {
            subtable.extend_from_slice(&body);
        }
        // Header, then the script list (one default script, one default
        // language), the feature list (`liga`), the lookup list (type 4)
        // and the subtable, each at the offset the one before it ends on.
        let mut out = Vec::new();
        push32(&mut out, 0x0001_0000);
        push16(&mut out, 10);
        push16(&mut out, 30);
        push16(&mut out, 44);
        push16(&mut out, 1);
        out.extend_from_slice(b"DFLT");
        push16(&mut out, 8);
        push16(&mut out, 4); // Script: its default language
        push16(&mut out, 0); // Script: no others
        push16(&mut out, 0); // language: lookup order
        push16(&mut out, 0xFFFF); // no required feature
        push16(&mut out, 1);
        push16(&mut out, 0);
        push16(&mut out, 1);
        out.extend_from_slice(b"liga");
        push16(&mut out, 8);
        push16(&mut out, 0); // Feature: no parameters
        push16(&mut out, 1);
        push16(&mut out, 0);
        push16(&mut out, 1);
        push16(&mut out, 4);
        push16(&mut out, 4); // Lookup: ligature substitution
        push16(&mut out, 0);
        push16(&mut out, 1);
        push16(&mut out, 8);
        out.extend_from_slice(&subtable);
        out
    }

    /// Version 0 layers.
    fn colr(&self) -> Vec<u8> {
        let mut bases: Vec<(u16, &Vec<(char, u16)>)> = self
            .layers
            .iter()
            .map(|(letter, layers)| (glyph(*letter), layers))
            .collect();
        bases.sort_unstable_by_key(|&(id, _)| id);
        let layer_total: usize = bases.iter().map(|(_, layers)| layers.len()).sum();
        let mut out = Vec::new();
        push16(&mut out, 0);
        push16(&mut out, bases.len() as u16);
        push32(&mut out, 14);
        push32(&mut out, 14 + 6 * bases.len() as u32);
        push16(&mut out, layer_total as u16);
        let mut first = 0u16;
        for (id, layers) in &bases {
            push16(&mut out, *id);
            push16(&mut out, first);
            push16(&mut out, layers.len() as u16);
            first += layers.len() as u16;
        }
        for (_, layers) in &bases {
            for &(letter, entry) in layers.iter() {
                push16(&mut out, glyph(letter));
                push16(&mut out, entry);
            }
        }
        out
    }

    /// One palette.
    fn cpal(&self) -> Vec<u8> {
        let mut out = Vec::new();
        push16(&mut out, 0);
        push16(&mut out, self.palette.len() as u16);
        push16(&mut out, 1);
        push16(&mut out, self.palette.len() as u16);
        push32(&mut out, 14);
        push16(&mut out, 0);
        for [r, g, b, a] in &self.palette {
            out.extend_from_slice(&[*b, *g, *r, *a]);
        }
        out
    }

    /// The strikes, each a picture per glyph that has one.
    fn sbix(&self) -> Vec<u8> {
        let glyphs = usize::from(glyph_count());
        let mut out = Vec::new();
        push16(&mut out, 1);
        push16(&mut out, 1);
        push32(&mut out, self.strikes.len() as u32);
        let mut bodies = Vec::new();
        for (ppem, pictures) in &self.strikes {
            let mut by_glyph: BTreeMap<u16, &Picture> = BTreeMap::new();
            for picture in pictures {
                by_glyph.insert(glyph(picture.letter), picture);
            }
            let mut body = Vec::new();
            push16(&mut body, *ppem);
            push16(&mut body, 72);
            let mut data = Vec::new();
            let start = 4 + 4 * (glyphs + 1);
            for id in 0..=glyphs {
                push32(&mut body, (start + data.len()) as u32);
                if let Some(picture) = by_glyph.get(&(id as u16)) {
                    data.extend_from_slice(&picture.x.to_be_bytes());
                    data.extend_from_slice(&picture.y.to_be_bytes());
                    data.extend_from_slice(b"png ");
                    data.extend_from_slice(&picture.png);
                }
            }
            body.extend_from_slice(&data);
            bodies.push(body);
        }
        let mut offset = 8 + 4 * bodies.len();
        for body in &bodies {
            push32(&mut out, offset as u32);
            offset += body.len();
        }
        for body in bodies {
            out.extend_from_slice(&body);
        }
        out
    }
}

/// The faces the app carries and the assembled ones, installed from files
/// as a project's fonts are; the files go when this does.
struct Installed {
    fonts: Fonts,
    files: Vec<PathBuf>,
}

impl Drop for Installed {
    fn drop(&mut self) {
        for file in &self.files {
            let _ = std::fs::remove_file(file);
        }
    }
}

fn install(faces: &[Vec<u8>]) -> Installed {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let mut fonts = Fonts::bundled();
    let mut files = Vec::new();
    for face in faces {
        let file = std::env::temp_dir().join(format!(
            "concat-text-emoji-{}-{}.ttf",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&file, face).expect("the face is written");
        assert!(fonts.add_file(&file), "the assembled face loads");
        files.push(file);
    }
    Installed { fonts, files }
}

/// A face that draws GRIN as a green square of `side` strike pixels at
/// `ppem`, its origin offset by `(x, y)`.
fn square(ppem: u16, side: u32, x: i16, y: i16) -> Recipe {
    Recipe {
        map: vec![(GRIN, 'ก')],
        strikes: vec![(
            ppem,
            vec![Picture {
                letter: 'ก',
                x,
                y,
                png: png(side, [0, 255, 0, 255]),
            }],
        )],
        ..Recipe::default()
    }
}

/// A face that draws GRIN as two layers, red under blue.
fn red_and_blue() -> Recipe {
    Recipe {
        map: vec![(GRIN, 'ก')],
        layers: vec![('ก', vec![('ข', 0), ('ค', 1)])],
        palette: vec![[255, 0, 0, 255], [0, 0, 255, 255]],
        ..Recipe::default()
    }
}

// ── bitmap strikes ──

/// A strike's picture is scaled from the strike's pixels per em to the
/// title's em: 32 pixels at 32 per em is a full em, at 64 per em half.
#[test]
fn a_strike_picture_is_scaled_from_its_strike_to_the_em() {
    for (ppem, want) in [(32, EM), (64, EM / 2.0)] {
        let fonts = install(&[square(ppem, 32, 0, 0).build()]);
        let frame = paint(&fonts.fonts, &title(&GRIN.to_string()));
        let span = extent(&frame, is_green).expect("the picture is painted");
        assert!(near(width_of(span), want, 2.0), "{ppem}: {span:?}");
        assert!(near(height_of(span), want, 2.0), "{ppem}: {span:?}");
    }
}

/// A picture's origin is the pen on the baseline: its bottom edge sits on
/// the baseline the letters stand on, and its origin offsets move it by
/// strike pixels scaled to the em, up and to the right.
#[test]
fn a_strike_picture_stands_on_the_baseline_by_its_origin() {
    let at = |x: i16, y: i16| {
        let fonts = install(&[square(32, 32, x, y).build()]);
        let frame = paint(&fonts.fonts, &title(&format!("H{GRIN}")));
        let letter = extent(&frame, is_white).expect("the H is painted");
        let picture = extent(&frame, is_green).expect("the picture is painted");
        (letter, picture)
    };
    let (letter, still) = at(0, 0);
    assert!(
        near(still[3] as f32, letter[3] as f32, 2.0),
        "picture bottom {still:?} on the H's baseline {letter:?}"
    );
    assert!(still[0] >= letter[2], "the picture follows the H");
    // Eight strike pixels at 32 per em is a quarter of an 80-pixel em.
    let (_, raised) = at(0, 8);
    assert!(near((still[1] - raised[1]) as f32, 20.0, 1.5), "{raised:?}");
    let (_, pushed) = at(8, 0);
    assert!(near((pushed[0] - still[0]) as f32, 20.0, 1.5), "{pushed:?}");
}

/// Where a glyph has both a strike picture and layers, the picture is
/// what lands.
#[test]
fn a_strike_picture_wins_over_layers() {
    let mut both = red_and_blue();
    both.strikes = square(32, 32, 0, 0).strikes;
    let fonts = install(&[both.build()]);
    let frame = paint(&fonts.fonts, &title(&GRIN.to_string()));
    assert!(count(&frame, is_green) > 1000);
    assert_eq!(count(&frame, is_red), 0);
    assert_eq!(count(&frame, is_blue), 0);
}

/// A face with strikes at two sizes draws the glyph at the same size
/// from either: the picture is an em whichever strike is used, and in
/// one of the strikes' colours.
#[test]
fn strikes_at_different_sizes_agree_on_the_size() {
    let face = Recipe {
        map: vec![(GRIN, 'ก')],
        strikes: vec![
            (
                16,
                vec![Picture {
                    letter: 'ก',
                    x: 0,
                    y: 0,
                    png: png(16, [255, 0, 0, 255]),
                }],
            ),
            (
                128,
                vec![Picture {
                    letter: 'ก',
                    x: 0,
                    y: 0,
                    png: png(128, [0, 0, 255, 255]),
                }],
            ),
        ],
        ..Recipe::default()
    };
    let fonts = install(&[face.build()]);
    let frame = paint(&fonts.fonts, &title(&GRIN.to_string()));
    let span = extent(&frame, is_saturated).expect("a picture is painted");
    assert!(near(width_of(span), EM, 2.0), "{span:?}");
    assert!(near(height_of(span), EM, 2.0), "{span:?}");
    assert!(count(&frame, is_red) + count(&frame, is_blue) > 4000);
    assert_eq!(count(&frame, is_white), 0, "not the fill");
}

// ── layers ──

/// Layers paint in their palette's colours, whatever the title's fill.
#[test]
fn layers_paint_in_their_palette_not_the_fill() {
    let fonts = install(&[red_and_blue().build()]);
    let white = paint(&fonts.fonts, &title(&GRIN.to_string()));
    assert!(count(&white, is_red) > 100, "the lower layer");
    assert!(count(&white, is_blue) > 100, "the upper layer");
    assert_eq!(count(&white, is_white), 0);
    let mut green = title(&GRIN.to_string());
    green.color = "#00ff00".to_owned();
    let green = paint(&fonts.fonts, &green);
    assert_eq!(count(&green, is_green), 0);
    assert_eq!(white, green, "the fill changes nothing");
}

/// A layer that asks for the text's own colour takes the fill - or the
/// colour of the run the emoji is in - and the fill's opacity with it.
#[test]
fn a_foreground_layer_takes_the_text_colour() {
    let face = Recipe {
        map: vec![(GRIN, 'ก')],
        layers: vec![('ก', vec![('ข', 0xFFFF), ('ค', 0)])],
        palette: vec![[255, 0, 0, 255]],
        ..Recipe::default()
    };
    let fonts = install(&[face.build()]);
    let mut style = title(&format!("a{GRIN}"));
    style.color = "#00ff00".to_owned();
    let filled = paint(&fonts.fonts, &style);
    assert!(count(&filled, is_green) > 100, "the fill's green");
    assert!(count(&filled, is_red) > 100, "the palette's red");

    style.runs = vec![TitleRun {
        start: 1,
        end: 2,
        color: Some("#ff00ff".to_owned()),
        ..TitleRun::default()
    }];
    let run = paint(&fonts.fonts, &style);
    assert!(count(&run, is_magenta) > 100, "the run's colour");
    assert!(count(&run, is_red) > 100);

    style.runs.clear();
    style.color = "#00ff0080".to_owned();
    let faint = paint(&fonts.fonts, &style);
    assert_eq!(count(&faint, is_green), 0, "no solid green");
    assert!(
        pixels(&faint).any(|(_, _, p)| p[1] > 180 && p[0] < 80 && (100..=160).contains(&p[3])),
        "half-opaque green"
    );
    assert!(count(&faint, is_red) > 100, "the palette's own stays solid");
}

// ── the title's styling ──

/// An emoji alone ignores the fill, its opacity, the outline and the
/// shadow: the frame is the same with all of them on.
#[test]
fn fill_outline_and_shadow_leave_an_emoji_alone() {
    for face in [square(32, 32, 0, 0), red_and_blue()] {
        let fonts = install(&[face.build()]);
        let plain = paint(&fonts.fonts, &title(&GRIN.to_string()));
        let mut dressed = title(&GRIN.to_string());
        dressed.color = "#ff000080".to_owned();
        dressed.stroke_width = 0.02;
        dressed.stroke_color = "#0000ff".to_owned();
        dressed.shadow = true;
        let dressed = paint(&fonts.fonts, &dressed);
        assert_eq!(plain, dressed);
    }
}

/// Beside an emoji, letters keep their outline and shadow, and the emoji
/// is laid over them untouched.
#[test]
fn letters_beside_an_emoji_keep_their_outline() {
    let fonts = install(&[square(64, 32, 0, 0).build()]);
    let words = format!("H{GRIN}H");
    let plain = paint(&fonts.fonts, &title(&words));
    let mut outlined = title(&words);
    outlined.stroke_width = 0.01;
    outlined.stroke_color = "#0000ff".to_owned();
    outlined.shadow = true;
    let outlined = paint(&fonts.fonts, &outlined);
    assert!(count(&outlined, is_blue) > 100, "the letters' outline");
    assert_eq!(count(&plain, is_blue), 0);
    assert_eq!(
        extent(&plain, is_green),
        extent(&outlined, is_green),
        "the picture does not move"
    );
    let (before, after) = (count(&plain, is_green), count(&outlined, is_green));
    assert!(before > 1000);
    assert!(
        after * 100 >= before * 98,
        "{after} of {before} still green"
    );
}

/// The plate goes under an emoji, and counts its advance.
#[test]
fn a_plate_goes_under_an_emoji() {
    let fonts = install(&[square(64, 32, 0, 0).build()]);
    let bare = paint(&fonts.fonts, &title(&GRIN.to_string()));
    let mut plated = title(&GRIN.to_string());
    plated.background = "#0000ff".to_owned();
    plated.background_padding_x = 0.02;
    plated.background_padding_y = 0.02;
    let plated = paint(&fonts.fonts, &plated);
    assert!(count(&plated, is_blue) > 1000, "the plate");
    let span = extent(&plated, is_green).expect("the picture over it");
    let (cx, cy) = ((span[0] + span[2]) / 2, (span[1] + span[3]) / 2);
    let middle = (cy as u32 * plated.width + cx as u32) as usize * 4;
    assert!(is_green(
        plated.rgba[middle..middle + 4].try_into().expect("a pixel")
    ));
    assert!(bare.block_width > 0);
    assert!(plated.block_width > bare.block_width);
    assert!(plated.block_height > bare.block_height);
}

/// Letter spacing goes between emoji as between letters.
#[test]
fn letter_spacing_goes_between_emoji() {
    let fonts = install(&[square(64, 32, 0, 0).build()]);
    let pair = format!("{GRIN}{GRIN}");
    let tight = paint(&fonts.fonts, &title(&pair));
    let mut loose = title(&pair);
    loose.tracking = 0.05; // 20 pixels
    let loose = paint(&fonts.fonts, &loose);
    let gap = loose.block_width as f32 - tight.block_width as f32;
    assert!(near(gap, 20.0, 1.0), "{gap}");
    let spread = width_of(extent(&loose, is_green).expect("painted"))
        - width_of(extent(&tight, is_green).expect("painted"));
    assert!(near(spread, 20.0, 1.5), "{spread}");
}

/// A run that scales its words scales an emoji in it.
#[test]
fn a_bigger_run_makes_a_bigger_emoji() {
    let fonts = install(&[square(32, 32, 0, 0).build()]);
    let mut style = title(&format!("{GRIN}a"));
    style.runs = vec![TitleRun {
        start: 0,
        end: 1,
        font_scale: Some(2.0),
        ..TitleRun::default()
    }];
    let frame = paint(&fonts.fonts, &style);
    let span = extent(&frame, is_green).expect("painted");
    assert!(near(width_of(span), 2.0 * EM, 3.0), "{span:?}");
    assert!(near(height_of(span), 2.0 * EM, 3.0), "{span:?}");
}

/// An emoji after a space is a word of its own, its box starting where
/// its picture does.
#[test]
fn an_emoji_is_a_word() {
    let fonts = install(&[square(64, 32, 0, 0).build()]);
    let frame = paint(&fonts.fonts, &title(&format!("ab {GRIN}")));
    assert_eq!(frame.words.len(), 2, "{:?}", frame.words);
    let span = extent(&frame, is_green).expect("painted");
    let word = frame.words[1];
    assert!(word.width > 0);
    assert!(
        near(word.x as f32, span[0] as f32, 1.5),
        "{word:?} {span:?}"
    );
}

/// An emoji's picture is ink like any letter's: past the frame's edge,
/// the canvas grows to hold it whole.
#[test]
fn an_emoji_past_the_frame_grows_the_canvas() {
    let fonts = install(&[square(32, 32, 0, 0).build()]);
    let frame =
        render_frame(&fonts.fonts, &title(&GRIN.to_string()), 60, TALL).expect("the title renders");
    assert!(frame.width > 60, "{}", frame.width);
    assert!(frame.overscan > 1.0);
    let span = extent(&frame, is_green).expect("painted");
    assert!(near(width_of(span), EM, 2.0), "whole: {span:?}");
}

// ── which face draws it ──

/// A face that paints its own colours is borrowed before one that only
/// has an outline, even when the title's weight matches the outline face
/// and not the colour one.
#[test]
fn a_colour_face_is_borrowed_before_a_heavier_plain_one() {
    let plain = Recipe {
        map: vec![(GRIN, 'ง')],
        weight: Some(900),
        ..Recipe::default()
    };
    let mut heavy = title(&format!("Go {GRIN}"));
    heavy.font_weight = 900.0;

    let only_plain = install(&[plain.build()]);
    let frame = paint(&only_plain.fonts, &heavy);
    assert_eq!(count(&frame, is_saturated), 0, "the outline in the fill");

    let both = install(&[plain.build(), red_and_blue().build()]);
    let frame = paint(&both.fonts, &heavy);
    assert!(count(&frame, is_red) > 100);
    assert!(count(&frame, is_blue) > 100);
}

/// A colour glyph that cannot be painted - a picture that will not
/// decode, a layer whose colour is not in the palette, layers with no
/// outline at all - is drawn as its outline in the title's fill, not
/// left out.
#[test]
fn an_unpaintable_colour_glyph_falls_back_to_its_outline() {
    let undecodable = Recipe {
        map: vec![(GRIN, 'ก')],
        strikes: vec![(
            32,
            vec![Picture {
                letter: 'ก',
                x: 0,
                y: 0,
                png: broken_png(32),
            }],
        )],
        ..Recipe::default()
    };
    let off_palette = Recipe {
        map: vec![(GRIN, 'ก')],
        layers: vec![('ก', vec![('ข', 7)])],
        palette: vec![[255, 0, 0, 255]],
        ..Recipe::default()
    };
    let blank = Recipe {
        map: vec![(GRIN, 'ก')],
        layers: vec![('ก', vec![(' ', 0)])],
        palette: vec![[255, 0, 0, 255]],
        ..Recipe::default()
    };
    for (name, face) in [
        ("undecodable", undecodable),
        ("off the palette", off_palette),
        ("blank layers", blank),
    ] {
        let fonts = install(&[face.build()]);
        let mut style = title(&GRIN.to_string());
        style.color = "#ff00ff".to_owned();
        let frame = paint(&fonts.fonts, &style);
        assert!(count(&frame, is_magenta) > 100, "{name}: the outline");
        assert_eq!(count(&frame, is_red), 0, "{name}");
        assert_eq!(count(&frame, is_green), 0, "{name}");
    }
}

/// Two emoji from the same colour face, side by side, each draw their
/// own picture.
#[test]
fn neighbouring_emoji_each_draw_their_own() {
    let face = Recipe {
        map: vec![(GRIN, 'ก'), (WINK, 'ข')],
        strikes: vec![(
            64,
            vec![
                Picture {
                    letter: 'ก',
                    x: 0,
                    y: 0,
                    png: png(32, [0, 255, 0, 255]),
                },
                Picture {
                    letter: 'ข',
                    x: 0,
                    y: 0,
                    png: png(32, [255, 0, 0, 255]),
                },
            ],
        )],
        ..Recipe::default()
    };
    let fonts = install(&[face.build()]);
    let frame = paint(&fonts.fonts, &title(&format!("{GRIN}{WINK}")));
    let green = extent(&frame, is_green).expect("the first");
    let red = extent(&frame, is_red).expect("the second");
    assert!(green[2] <= red[0] + 1, "in order: {green:?} {red:?}");
}

/// With no colour face anywhere, a title with an emoji still renders, its
/// letters in the fill and nothing invented in colour.
#[test]
fn with_no_colour_face_an_emoji_title_still_renders() {
    let fonts = Fonts::bundled();
    let frame = paint(&fonts, &title(&format!("a {GRIN} \u{2764}\u{FE0F} b")));
    assert!(frame.block_width > 0);
    assert!(count(&frame, is_white) > 50, "the letters");
    assert_eq!(count(&frame, is_saturated), 0);
}

/// A face that joins a family, a skin tone, a flag and a heart asked for
/// in colour into one glyph each, the way an emoji face does, and draws
/// each in a colour of its own.
fn joining_face() -> Recipe {
    let strike = |letter, colour| Picture {
        letter,
        x: 0,
        y: 0,
        png: png(64, colour),
    };
    Recipe {
        map: vec![
            (GRIN, 'ก'),
            ('\u{1F468}', 'ข'),
            ('\u{1F469}', 'ค'),
            ('\u{1F467}', 'ฆ'),
            ('\u{200D}', 'ง'),
            ('\u{1F44D}', 'จ'),
            ('\u{1F3FD}', 'ฉ'),
            ('\u{1F1EF}', 'ช'),
            ('\u{1F1F5}', 'ซ'),
        ],
        joins: vec![
            (vec!['ข', 'ง', 'ค', 'ง', 'ฆ'], 'ฮ'),
            (vec!['จ', 'ฉ'], 'ฎ'),
            (vec!['ช', 'ซ'], 'ฏ'),
        ],
        strikes: vec![(
            64,
            vec![
                strike('ก', [0, 255, 0, 255]),
                strike('ฮ', [255, 0, 0, 255]),
                strike('ฎ', [0, 0, 255, 255]),
                strike('ฏ', [255, 0, 255, 255]),
            ],
        )],
        ..Recipe::default()
    }
}

/// A family, a skin tone and a flag, each made of several characters, are
/// shaped by the face into one glyph and drawn as one picture about one
/// emoji wide: not as its parts side by side, and not as an outline.
#[test]
fn a_sequence_the_face_joins_draws_as_one_picture() {
    let fonts = install(&[joining_face().build()]);
    let width = |text: &str| paint(&fonts.fonts, &title(text)).block_width as f32;
    let one = width("\u{1F600}");
    for (sequence, wanted) in [
        (
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
            is_red as fn(Rgba) -> bool,
        ),
        ("\u{1F44D}\u{1F3FD}", is_blue),
        ("\u{1F1EF}\u{1F1F5}", is_magenta),
    ] {
        let frame = paint(&fonts.fonts, &title(sequence));
        let wide = frame.block_width as f32;
        assert!(
            wide > 0.0 && wide <= one * 1.3,
            "{sequence}: {wide} vs {one}"
        );
        assert!(count(&frame, wanted) > 50, "{sequence} in its colour");
        assert_eq!(count(&frame, is_white), 0, "{sequence} left no outline");
    }
}

// ── the machine's own colour emoji face ──

/// The machine's faces, loaded once for every case below.
fn system() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(Fonts::new)
}

/// Whether the machine has a colour face that draws `emoji` in colour.
fn draws_in_colour(emoji: &str) -> bool {
    count(&paint(system(), &title(emoji)), is_saturated) > 50
}

/// Whether the machine has a colour emoji face to test with. Where it has
/// Apple's, whose pictures are bitmap strikes, it must be drawn in colour:
/// that one is never a reason to skip.
fn has_colour_emoji() -> bool {
    let drawn = draws_in_colour("\u{1F600}");
    let apple = system()
        .families()
        .iter()
        .any(|family| family == "Apple Color Emoji");
    assert!(
        drawn || !apple,
        "Apple Color Emoji is installed but drew no colour"
    );
    drawn
}

/// Whether `text` is painted as the face's own picture: the same pixels
/// whatever the title's fill, where an outline falls back to the fill.
fn is_a_picture(fonts: &Fonts, text: &str) -> bool {
    let mut other = title(text);
    other.color = "#ff0000".to_owned();
    let (white, red) = (paint(fonts, &title(text)), paint(fonts, &other));
    white.rgba.iter().any(|&byte| byte != 0) && white.rgba == red.rgba
}

/// How many glyphs the face set `text` in. Letter spacing falls between
/// glyphs and after none, so the width a spacing of 20 pixels adds is 20
/// for each glyph after the first: one glyph means the face joined the
/// sequence, more mean it drew its parts.
fn glyphs_of(fonts: &Fonts, text: &str) -> usize {
    let mut loose = title(text);
    loose.tracking = 0.05; // 20 pixels
    let gap =
        paint(fonts, &loose).block_width as f32 - paint(fonts, &title(text)).block_width as f32;
    (gap / 20.0).round() as usize + 1
}

/// Sequences joined into one picture by the face - a family joined with
/// zero-width joiners, a skin tone, a flag, a heart asked for in colour -
/// draw as one emoji, about one emoji wide, as the face's picture.
///
/// Which sequences a face has a picture for is the face's to say: Windows'
/// has no flags, and Noto Color Emoji (Linux, Android) draws a family as a
/// grey picture, so the picture is told apart from an outline by ignoring
/// the fill, not by being coloured. Whether the face joined a sequence is
/// read from how many glyphs it was set in. A joined sequence must be that
/// picture. One the face did not join is its parts, each drawn: a flag
/// from a face with no flags is its two letters, set in the title's fill.
/// Apple's face has all four, so there each must be one glyph and in
/// colour. Concat joining what any face offers is also settled with an
/// assembled face, in `a_sequence_the_face_joins_draws_as_one_picture`.
#[test]
fn emoji_sequences_draw_as_one_picture() {
    if !has_colour_emoji() {
        eprintln!("no colour emoji face on this machine; skipped");
        return;
    }
    let apple = system()
        .families()
        .iter()
        .any(|family| family == "Apple Color Emoji");
    let width = |text: &str| paint(system(), &title(text)).block_width as f32;
    let one = width("\u{1F600}");
    for sequence in [
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        "\u{1F44D}\u{1F3FD}",
        "\u{1F1EF}\u{1F1F5}",
        "\u{2764}\u{FE0F}",
    ] {
        let wide = width(sequence);
        let glyphs = glyphs_of(system(), sequence);
        let picture = is_a_picture(system(), sequence);
        let measured = format!(
            "{sequence}: {glyphs} glyphs, {:.2} em wide (one emoji {:.2} em), \
             the fill {} the pixels, apple {apple}",
            wide / EM,
            one / EM,
            if picture { "left" } else { "changed" },
        );
        assert!(wide > 0.0, "it renders - {measured}");
        if apple {
            assert_eq!(glyphs, 1, "one glyph - {measured}");
            assert!(draws_in_colour(sequence), "in colour - {measured}");
        } else if glyphs == 1 {
            assert!(picture, "one glyph is a picture - {measured}");
            assert!(wide <= one * 1.3, "one emoji wide - {measured}");
        } else {
            // The face has no picture for it: its parts, drawn in the fill.
            let characters = sequence.chars().count();
            assert!(
                glyphs <= characters,
                "no more glyphs than characters - {measured}"
            );
            assert!(!picture, "its parts are letters in the fill - {measured}");
        }
    }
}

/// The machine's emoji ignore the fill, outline and shadow too.
#[test]
fn system_emoji_ignore_fill_outline_and_shadow() {
    if !has_colour_emoji() {
        eprintln!("no colour emoji face on this machine; skipped");
        return;
    }
    let plain = paint(system(), &title("\u{1F600}"));
    let mut dressed = title("\u{1F600}");
    dressed.color = "#ff0000".to_owned();
    dressed.stroke_width = 0.02;
    dressed.stroke_color = "#0000ff".to_owned();
    dressed.shadow = true;
    assert_eq!(plain, paint(system(), &dressed));
}

/// A heavy italic title still borrows the colour face for its emoji.
#[test]
fn a_heavy_italic_title_keeps_emoji_in_colour() {
    if !has_colour_emoji() {
        eprintln!("no colour emoji face on this machine; skipped");
        return;
    }
    let mut style = title("Go \u{1F680}");
    style.font_weight = 900.0;
    style.italic = true;
    assert!(count(&paint(system(), &style), is_saturated) > 50);
}

/// The machine's emoji are about an em tall, and twice the size is twice
/// as tall.
#[test]
fn system_emoji_are_sized_to_the_em() {
    if !has_colour_emoji() {
        eprintln!("no colour emoji face on this machine; skipped");
        return;
    }
    let tall = |size: f64| {
        let mut style = title("\u{1F600}");
        style.font_size = size;
        height_of(extent(&paint(system(), &style), is_saturated).expect("painted"))
    };
    let (one, two) = (tall(SIZE), tall(2.0 * SIZE));
    assert!((0.6 * EM..=1.4 * EM).contains(&one), "{one}");
    assert!((1.8..=2.2).contains(&(two / one)), "{one} then {two}");
}

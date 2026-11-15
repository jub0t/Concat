// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How colour glyph pictures are made and kept, and the newer colour
//! layers - gradients, clips, transforms, blends - drawn. The faces are
//! assembled here from the bundled Thai face, so every case runs on a
//! machine with no fonts of its own.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{Key, Picture, Pictures, decode, footprints, shrink};
use crate::{Align, BUNDLED, BUNDLED_FAMILY, Fonts, NOTO_SANS_THAI, RenderedFrame, TitleStyle};

const WIDE: u32 = 640;
const TALL: u32 = 400;
/// An 80-pixel em on that frame.
const SIZE: f64 = 0.2;

const GRIN: char = '\u{1F600}';

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

fn draw(fonts: &Fonts, style: &TitleStyle) -> RenderedFrame {
    crate::render_frame(fonts, style, WIDE, TALL).expect("the title renders")
}

// ── pixels ──

type Rgba = [u8; 4];

fn at(frame: &RenderedFrame, x: i64, y: i64) -> Rgba {
    let i = (y as usize * frame.width as usize + x as usize) * 4;
    frame.rgba[i..i + 4].try_into().expect("a pixel")
}

fn solid(p: Rgba) -> bool {
    p[3] > 200
}
fn red(p: Rgba) -> bool {
    solid(p) && p[0] > 180 && p[1] < 80 && p[2] < 80
}
fn green(p: Rgba) -> bool {
    solid(p) && p[1] > 180 && p[0] < 80 && p[2] < 80
}
fn blue(p: Rgba) -> bool {
    solid(p) && p[2] > 180 && p[0] < 80 && p[1] < 80
}
fn magenta(p: Rgba) -> bool {
    solid(p) && p[0] > 180 && p[2] > 180 && p[1] < 80
}
fn white(p: Rgba) -> bool {
    solid(p) && p[0] > 200 && p[1] > 200 && p[2] > 200
}
fn reddish(p: Rgba) -> bool {
    solid(p) && i32::from(p[0]) > i32::from(p[2]) + 60
}
fn bluish(p: Rgba) -> bool {
    solid(p) && i32::from(p[2]) > i32::from(p[0]) + 60
}
fn coloured(p: Rgba) -> bool {
    let (high, low) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
    solid(p) && high - low > 60
}

fn each(frame: &RenderedFrame) -> impl Iterator<Item = (i64, i64, Rgba)> + '_ {
    frame.rgba.chunks_exact(4).enumerate().map(|(i, p)| {
        let i = i as i64;
        let w = i64::from(frame.width);
        (i % w, i / w, [p[0], p[1], p[2], p[3]])
    })
}

fn count(frame: &RenderedFrame, wanted: fn(Rgba) -> bool) -> usize {
    each(frame).filter(|&(_, _, p)| wanted(p)).count()
}

/// Left, top, right and bottom, the last two exclusive.
fn extent(frame: &RenderedFrame, wanted: fn(Rgba) -> bool) -> Option<[i64; 4]> {
    each(frame)
        .filter(|&(_, _, p)| wanted(p))
        .fold(None, |held, (x, y, _)| {
            Some(match held {
                None => [x, y, x + 1, y + 1],
                Some([l, t, r, b]) => [l.min(x), t.min(y), r.max(x + 1), b.max(y + 1)],
            })
        })
}

fn mean_x(frame: &RenderedFrame, wanted: fn(Rgba) -> bool) -> f64 {
    let (sum, n) = each(frame)
        .filter(|&(_, _, p)| wanted(p))
        .fold((0.0, 0.0), |(sum, n), (x, _, _)| (sum + x as f64, n + 1.0));
    sum / n
}

// ── faces, assembled ──

fn push16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn push24(out: &mut Vec<u8>, value: usize) {
    out.extend_from_slice(&(value as u32).to_be_bytes()[1..]);
}

fn push32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn set24(out: &mut [u8], at: usize, value: usize) {
    out[at..at + 3].copy_from_slice(&(value as u32).to_be_bytes()[1..]);
}

fn set32(out: &mut [u8], at: usize, value: usize) {
    out[at..at + 4].copy_from_slice(&(value as u32).to_be_bytes());
}

/// A number in the font's 2.14 fixed point.
fn f2(value: f32) -> u16 {
    ((value * 16384.0).round() as i16) as u16
}

fn thai() -> ttf_parser::Face<'static> {
    ttf_parser::Face::parse(NOTO_SANS_THAI, 0).expect("the Thai face parses")
}

/// The Thai face's glyph for `letter`: the outline an assembled glyph uses.
fn letter(letter: char) -> u16 {
    thai().glyph_index(letter).expect("the Thai face has it").0
}

fn tables_of(font: &[u8]) -> BTreeMap<[u8; 4], Vec<u8>> {
    let word =
        |at: usize| u32::from_be_bytes(font[at..at + 4].try_into().expect("4 bytes")) as usize;
    let count = usize::from(u16::from_be_bytes([font[4], font[5]]));
    (0..count)
        .map(|index| {
            let record = 12 + 16 * index;
            let tag: [u8; 4] = font[record..record + 4].try_into().expect("a tag");
            (
                tag,
                font[word(record + 8)..word(record + 8) + word(record + 12)].to_vec(),
            )
        })
        .collect()
}

fn font_of(tables: &BTreeMap<[u8; 4], Vec<u8>>) -> Vec<u8> {
    let count = tables.len();
    let log = (usize::BITS - 1 - count.leading_zeros()) as usize;
    let range = 16 << log;
    let mut out = Vec::new();
    push32(&mut out, 0x0001_0000);
    for value in [count, range, log, count * 16 - range] {
        push16(&mut out, value as u16);
    }
    let mut offset = 12 + 16 * count;
    for (tag, data) in tables {
        out.extend_from_slice(tag);
        push32(&mut out, 0);
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

fn png(side: u32, colour: Rgba) -> Vec<u8> {
    let mut pixmap = tiny_skia::Pixmap::new(side, side).expect("a pixmap");
    pixmap.fill(tiny_skia::Color::from_rgba8(
        colour[0], colour[1], colour[2], colour[3],
    ));
    pixmap.encode_png().expect("encodes")
}

/// One paint of a newer colour glyph, as the font stores it.
enum Node {
    /// The letter's outline, filled with what is inside.
    Glyph(char, Box<Node>),
    /// A palette entry, or 0xFFFF for the text's own colour.
    Solid(u16),
    /// `x0, y0, x1, y1, x2, y2` and the stops as offset and palette entry.
    Linear([i16; 6], Vec<(f32, u16)>),
    /// `x0, y0, r0, x1, y1, r1` and the stops.
    Radial([i16; 6], Vec<(f32, u16)>),
    /// The centre, the start and end angles as stored, and the stops.
    Sweep([i16; 2], f32, f32, Vec<(f32, u16)>),
    /// What is inside, scaled about the origin.
    Scale(f32, Box<Node>),
    /// Source, mode and backdrop.
    Composite(Box<Node>, u8, Box<Node>),
}

impl Node {
    fn glyph(of: char, inside: Node) -> Node {
        Node::Glyph(of, Box::new(inside))
    }

    /// Writes the paint and everything under it, returning where it starts;
    /// offsets are from the start of the paint that holds them.
    fn write(&self, out: &mut Vec<u8>) -> usize {
        let start = out.len();
        match self {
            Node::Glyph(of, inside) => {
                out.push(10);
                let link = out.len();
                push24(out, 0);
                push16(out, letter(*of));
                let child = inside.write(out);
                set24(out, link, child - start);
            }
            Node::Solid(entry) => {
                out.push(2);
                push16(out, *entry);
                push16(out, f2(1.0));
            }
            Node::Linear(points, stops) | Node::Radial(points, stops) => {
                out.push(if matches!(self, Node::Linear(..)) {
                    4
                } else {
                    6
                });
                let link = out.len();
                push24(out, 0);
                for point in points {
                    push16(out, *point as u16);
                }
                let line = colour_line(stops, out);
                set24(out, link, line - start);
            }
            Node::Sweep(centre, from, to, stops) => {
                out.push(8);
                let link = out.len();
                push24(out, 0);
                push16(out, centre[0] as u16);
                push16(out, centre[1] as u16);
                push16(out, f2(*from));
                push16(out, f2(*to));
                let line = colour_line(stops, out);
                set24(out, link, line - start);
            }
            Node::Scale(by, inside) => {
                out.push(16);
                let link = out.len();
                push24(out, 0);
                push16(out, f2(*by));
                push16(out, f2(*by));
                let child = inside.write(out);
                set24(out, link, child - start);
            }
            Node::Composite(source, mode, backdrop) => {
                out.push(32);
                let source_link = out.len();
                push24(out, 0);
                out.push(*mode);
                let backdrop_link = out.len();
                push24(out, 0);
                let at = source.write(out);
                set24(out, source_link, at - start);
                let at = backdrop.write(out);
                set24(out, backdrop_link, at - start);
            }
        }
        start
    }
}

fn colour_line(stops: &[(f32, u16)], out: &mut Vec<u8>) -> usize {
    let start = out.len();
    out.push(0); // pad
    push16(out, stops.len() as u16);
    for &(offset, entry) in stops {
        push16(out, f2(offset));
        push16(out, entry);
        push16(out, f2(1.0));
    }
    start
}

/// A colour bitmap in a `CBDT` strike: one glyph, PNG or BGRA.
struct Cbdt {
    ppem: u8,
    of: char,
    side: u8,
    bearing: (i8, i8),
    /// A PNG, or `side` by `side` premultiplied BGRA pixels.
    png: bool,
    data: Vec<u8>,
}

#[derive(Default)]
struct Recipe {
    /// Code points and the Thai letter whose glyph draws each.
    map: Vec<(char, char)>,
    /// `sbix` strikes: pixels per em, and the letter whose glyph has a
    /// picture there, at the pen.
    sbix: Vec<(u16, char, Vec<u8>)>,
    cbdt: Option<Cbdt>,
    /// Newer colour glyphs: the base letter and its paint.
    colr: Vec<(char, Node)>,
    /// One base glyph's clip box: xMin, yMin, xMax, yMax.
    clip: Option<(char, [i16; 4])>,
    palette: Vec<Rgba>,
    weight: Option<u16>,
}

impl Recipe {
    fn build(&self) -> Vec<u8> {
        let mut tables = tables_of(NOTO_SANS_THAI);
        tables.insert(*b"cmap", self.cmap());
        if !self.sbix.is_empty() {
            tables.insert(*b"sbix", self.sbix());
        }
        if let Some(cbdt) = &self.cbdt {
            let (cblc, cbdt) = cbdt.tables();
            tables.insert(*b"CBLC", cblc);
            tables.insert(*b"CBDT", cbdt);
        }
        if !self.colr.is_empty() {
            tables.insert(*b"COLR", self.colr());
            tables.insert(*b"CPAL", self.cpal());
        }
        if let Some(weight) = self.weight {
            tables.get_mut(b"OS/2").expect("an OS/2 table")[4..6]
                .copy_from_slice(&weight.to_be_bytes());
        }
        font_of(&tables)
    }

    fn cmap(&self) -> Vec<u8> {
        let mut groups: Vec<(u32, u16)> = self
            .map
            .iter()
            .map(|&(ch, of)| (ch as u32, letter(of)))
            .collect();
        groups.sort_unstable();
        let mut out = Vec::new();
        for value in [0, 1, 3, 10] {
            push16(&mut out, value);
        }
        push32(&mut out, 12);
        push16(&mut out, 12);
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

    fn sbix(&self) -> Vec<u8> {
        let glyphs = usize::from(thai().number_of_glyphs());
        let bodies: Vec<Vec<u8>> = self
            .sbix
            .iter()
            .map(|(ppem, of, png)| {
                let drawn = letter(*of);
                let mut body = Vec::new();
                push16(&mut body, *ppem);
                push16(&mut body, 72);
                let mut data = Vec::new();
                for id in 0..=glyphs {
                    push32(&mut body, (4 + 4 * (glyphs + 1) + data.len()) as u32);
                    if id == usize::from(drawn) {
                        // Origin offsets of nothing: the pen.
                        data.extend_from_slice(&[0; 4]);
                        data.extend_from_slice(b"png ");
                        data.extend_from_slice(png);
                    }
                }
                body.extend_from_slice(&data);
                body
            })
            .collect();
        let mut out = Vec::new();
        push16(&mut out, 1);
        push16(&mut out, 1);
        push32(&mut out, bodies.len() as u32);
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

    fn colr(&self) -> Vec<u8> {
        let mut bases: Vec<(u16, &Node)> = self
            .colr
            .iter()
            .map(|(of, node)| (letter(*of), node))
            .collect();
        bases.sort_unstable_by_key(|&(id, _)| id);
        let mut list = Vec::new();
        push32(&mut list, bases.len() as u32);
        let records = list.len();
        for &(id, _) in &bases {
            push16(&mut list, id);
            push32(&mut list, 0);
        }
        for (index, (_, node)) in bases.iter().enumerate() {
            let at = node.write(&mut list);
            set32(&mut list, records + index * 6 + 2, at);
        }
        let mut out = Vec::new();
        push16(&mut out, 1); // version
        push16(&mut out, 0); // no version 0 base glyphs
        push32(&mut out, 0);
        push32(&mut out, 0);
        push16(&mut out, 0);
        push32(&mut out, 34); // the base glyph list
        push32(&mut out, 0); // no layer list
        let clip_at = 34 + list.len();
        push32(
            &mut out,
            if self.clip.is_some() {
                clip_at as u32
            } else {
                0
            },
        );
        push32(&mut out, 0);
        push32(&mut out, 0);
        out.extend_from_slice(&list);
        if let Some((of, [x0, y0, x1, y1])) = self.clip {
            out.push(1);
            push32(&mut out, 1);
            push16(&mut out, letter(of));
            push16(&mut out, letter(of));
            push24(&mut out, 12);
            out.push(1);
            for value in [x0, y0, x1, y1] {
                push16(&mut out, value as u16);
            }
        }
        out
    }

    fn cpal(&self) -> Vec<u8> {
        let palette = if self.palette.is_empty() {
            vec![[0, 0, 0, 255]]
        } else {
            self.palette.clone()
        };
        let mut out = Vec::new();
        push16(&mut out, 0);
        push16(&mut out, palette.len() as u16);
        push16(&mut out, 1);
        push16(&mut out, palette.len() as u16);
        push32(&mut out, 14);
        push16(&mut out, 0);
        for [r, g, b, a] in palette {
            out.extend_from_slice(&[b, g, r, a]);
        }
        out
    }
}

impl Cbdt {
    fn tables(&self) -> (Vec<u8>, Vec<u8>) {
        let id = letter(self.of);
        let mut glyph = vec![
            self.side,
            self.side,
            self.bearing.0 as u8,
            self.bearing.1 as u8,
            self.side,
        ];
        if self.png {
            push32(&mut glyph, self.data.len() as u32);
        }
        glyph.extend_from_slice(&self.data);

        let mut cblc = Vec::new();
        push16(&mut cblc, 3);
        push16(&mut cblc, 0);
        push32(&mut cblc, 1);
        push32(&mut cblc, 56); // the index subtable array
        push32(&mut cblc, 24);
        push32(&mut cblc, 1);
        push32(&mut cblc, 0);
        cblc.extend_from_slice(&[0; 24]); // line metrics
        push16(&mut cblc, id);
        push16(&mut cblc, id);
        cblc.extend_from_slice(&[self.ppem, self.ppem, 32, 1]);
        push16(&mut cblc, id);
        push16(&mut cblc, id);
        push32(&mut cblc, 8);
        push16(&mut cblc, 1); // offsets, four bytes each
        push16(&mut cblc, if self.png { 17 } else { 1 });
        push32(&mut cblc, 4); // the glyph's data in CBDT
        push32(&mut cblc, 0);
        push32(&mut cblc, glyph.len() as u32);

        let mut cbdt = Vec::new();
        push16(&mut cbdt, 3);
        push16(&mut cbdt, 0);
        cbdt.extend_from_slice(&glyph);
        (cblc, cbdt)
    }
}

/// The bundled faces and these, installed from files as a project's are.
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
            "concat-text-colour-{}-{}.ttf",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&file, face).expect("the face is written");
        assert!(fonts.add_file(&file), "the assembled face loads");
        files.push(file);
    }
    Installed { fonts, files }
}

/// A face drawing GRIN as one green square in an `sbix` strike.
fn square(ppem: u16, side: u32) -> Recipe {
    Recipe {
        map: vec![(GRIN, 'ก')],
        sbix: vec![(ppem, 'ก', png(side, [0, 255, 0, 255]))],
        ..Recipe::default()
    }
}

/// A face drawing GRIN with the newer layers: `paint` alone.
fn newer(paint: Node) -> Recipe {
    Recipe {
        map: vec![(GRIN, 'ก')],
        colr: vec![('ก', paint)],
        palette: vec![[255, 0, 0, 255], [0, 0, 255, 255], [0, 255, 0, 255]],
        ..Recipe::default()
    }
}

const RED: u16 = 0;
const BLUE: u16 = 1;
const GREEN: u16 = 2;

/// The Thai glyph's outline bounds, in font units.
fn bounds(of: char) -> ttf_parser::Rect {
    thai()
        .glyph_bounding_box(ttf_parser::GlyphId(letter(of)))
        .expect("an outline")
}

// ── pictures, made once ──

/// A title of the same emoji many times over, painted again and again,
/// decodes its strike once: once in all, not once per emoji per paint.
/// The strike is bigger than the em, so its shrunk picture is kept too.
#[test]
fn repeated_emoji_are_decoded_once() {
    for (ppem, side) in [(128, 128), (32, 32)] {
        let fonts = install(&[square(ppem, side).build()]);
        let style = title(&GRIN.to_string().repeat(12));
        let first = draw(&fonts.fonts, &style);
        assert!(
            count(&first, green) > 10 * 4000,
            "{ppem}: every emoji drawn"
        );
        for _ in 0..4 {
            assert_eq!(draw(&fonts.fonts, &style), first);
        }
        assert_eq!(
            fonts.fonts.pictures.decodes(),
            1,
            "{ppem}: 12 emoji, 5 paints"
        );
        assert!(fonts.fonts.pictures.bytes() > 0);
    }
}

/// Threads painting with one set of faces share its pictures and paint the
/// same title the same.
#[test]
fn threads_share_the_pictures() {
    let fonts = install(&[square(128, 128).build()]);
    let style = title(&format!("{GRIN} {GRIN} {GRIN}"));
    let alone = draw(&fonts.fonts, &style);
    std::thread::scope(|scope| {
        let painters: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    (0..3)
                        .map(|_| draw(&fonts.fonts, &style))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for painter in painters {
            for frame in painter.join().expect("the thread paints") {
                assert_eq!(frame, alone);
            }
        }
    });
    assert_eq!(
        fonts.fonts.pictures.decodes(),
        1,
        "decoded before the threads began"
    );
}

/// The strike drawn from is the smallest at least an em, not the largest.
#[test]
fn the_strike_nearest_above_the_em_is_drawn() {
    let face = Recipe {
        map: vec![(GRIN, 'ก')],
        sbix: vec![
            (128, 'ก', png(128, [0, 0, 255, 255])),
            (512, 'ก', png(512, [255, 0, 0, 255])),
        ],
        ..Recipe::default()
    };
    let fonts = install(&[face.build()]);
    let small = draw(&fonts.fonts, &title(&GRIN.to_string()));
    assert!(
        count(&small, blue) > 4000 && count(&small, red) == 0,
        "80 px: the 128 strike"
    );
    let mut big = title(&GRIN.to_string());
    big.font_size = 0.4;
    let big = draw(&fonts.fonts, &big);
    assert!(
        count(&big, red) > 16_000 && count(&big, blue) == 0,
        "160 px: the 512 strike"
    );
}

/// Shrinking averages: fine stripes come out an even half tone, not
/// whichever stripe a sample landed on, and each new pixel's shares of the
/// old ones add up to one.
#[test]
fn a_shrunk_picture_is_the_average_of_what_it_covers() {
    let mut stripes = tiny_skia::Pixmap::new(160, 160).expect("a pixmap");
    for (index, pixel) in stripes.data_mut().chunks_exact_mut(4).enumerate() {
        if index % 2 == 0 {
            pixel.copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    let small = shrink(&stripes, 24, 24).expect("shrinks");
    assert_eq!((small.width(), small.height()), (24, 24));
    for pixel in small.pixels() {
        assert!((110..=145).contains(&pixel.alpha()), "{}", pixel.alpha());
    }
    for taps in footprints(160, 24) {
        let total: f32 = taps.iter().map(|&(_, share)| share).sum();
        assert!((total - 1.0).abs() < 1e-4, "{total}");
    }
    assert!(shrink(&stripes, 160, 160).is_none(), "nothing to shrink");
    assert!(shrink(&stripes, 200, 100).is_none(), "wider is not smaller");
}

/// The store keeps no more bytes than it is given, and lets the longest
/// unused pictures go first.
#[test]
fn kept_pictures_are_bounded_by_bytes() {
    let picture = || Picture::at_pen(tiny_skia::Pixmap::new(32, 32).expect("a pixmap"));
    let each = 32 * 32 * 4 + super::ENTRY_BYTES;
    let pictures = Pictures::holding(each * 10);
    let key = |glyph: u16| Key::Strike {
        face: super::FaceKey::of(&[], 0),
        glyph,
        ppem: 32,
    };
    for glyph in 0..50 {
        pictures.keep(key(glyph), Some(picture()));
        // The first is wanted all along.
        assert!(pictures.lookup(key(0)).is_some(), "{glyph}");
        assert!(pictures.bytes() <= each * 10);
    }
    assert!(pictures.lookup(key(1)).is_none(), "long unused, gone");
    assert!(pictures.lookup(key(49)).is_some(), "the newest kept");
    // Too big for the store at all: drawn, not kept.
    let huge = Picture::at_pen(tiny_skia::Pixmap::new(256, 256).expect("a pixmap"));
    pictures.keep(key(99), Some(huge));
    assert!(pictures.lookup(key(99)).is_none());
}

// ── strikes in other tables ──

/// A `CBDT` picture - Android's and Linux's emoji - sits and sizes as an
/// `sbix` one does: its bearings put the same box in the same place.
#[test]
fn cbdt_pictures_sit_where_sbix_ones_do() {
    let sbix = install(&[square(32, 32).build()]);
    let cbdt = Recipe {
        map: vec![(GRIN, 'ก')],
        cbdt: Some(Cbdt {
            ppem: 32,
            of: 'ก',
            side: 32,
            // From the pen to the top-left corner, y up: a 32-pixel
            // picture standing on the baseline.
            bearing: (0, 32),
            png: true,
            data: png(32, [0, 255, 0, 255]),
        }),
        ..Recipe::default()
    };
    let cbdt = install(&[cbdt.build()]);
    let words = format!("H{GRIN}");
    let one = extent(&draw(&sbix.fonts, &title(&words)), green).expect("sbix drawn");
    let other = extent(&draw(&cbdt.fonts, &title(&words)), green).expect("CBDT drawn");
    for side in 0..4 {
        assert!((one[side] - other[side]).abs() <= 1, "{one:?} {other:?}");
    }
}

/// A BGRA strike is read as such - its red is red, not blue - and one whose
/// bytes do not fill its stated size is no picture at all.
#[test]
fn bgra_strikes_are_swapped_and_their_length_checked() {
    let side = 16u8;
    let face = Recipe {
        map: vec![(GRIN, 'ก')],
        cbdt: Some(Cbdt {
            ppem: 16,
            of: 'ก',
            side,
            bearing: (0, 16),
            png: false,
            data: [0, 0, 255, 255].repeat(usize::from(side) * usize::from(side)),
        }),
        ..Recipe::default()
    };
    let fonts = install(&[face.build()]);
    let frame = draw(&fonts.fonts, &title(&GRIN.to_string()));
    assert!(count(&frame, red) > 4000);
    assert_eq!(count(&frame, blue), 0);

    let short = [0u8, 0, 255, 255].repeat(10);
    let image = ttf_parser::RasterGlyphImage {
        x: 0,
        y: 0,
        width: 4,
        height: 4,
        pixels_per_em: 4,
        format: ttf_parser::RasterImageFormat::BitmapPremulBgra32,
        data: &short,
    };
    assert!(decode(&image).is_none());
    let whole = [0u8, 0, 255, 255].repeat(16);
    let image = ttf_parser::RasterGlyphImage {
        data: &whole,
        ..image
    };
    let pixels = decode(&image).expect("decodes");
    assert_eq!(&pixels.data()[..4], &[255, 0, 0, 255]);
}

// ── the newer colour layers ──

/// A gradient glyph paints its gradient - red at one end, blue at the
/// other, in the order it runs - and not the fill.
#[test]
fn a_gradient_glyph_paints_its_gradient() {
    let edge = bounds('ข');
    let across = [edge.x_min, 0, edge.x_max, 0, edge.x_min, 1000];
    let face = newer(Node::glyph(
        'ข',
        Node::Linear(across, vec![(0.0, RED), (1.0, BLUE)]),
    ));
    let fonts = install(&[face.build()]);
    let frame = draw(&fonts.fonts, &title(&GRIN.to_string()));
    assert!(count(&frame, reddish) > 200 && count(&frame, bluish) > 200);
    assert!(
        mean_x(&frame, reddish) + 10.0 < mean_x(&frame, bluish),
        "red on the left"
    );
    assert_eq!(count(&frame, white), 0, "not the fill");
    let mut green_fill = title(&GRIN.to_string());
    green_fill.color = "#00ff00".to_owned();
    green_fill.stroke_width = 0.02;
    green_fill.shadow = true;
    assert_eq!(
        draw(&fonts.fonts, &green_fill),
        frame,
        "the look changes nothing"
    );
}

/// A face of newer layers is a colour face: borrowed before a plain face,
/// even a heavy title's.
#[test]
fn a_newer_colour_face_is_borrowed_before_a_plain_one() {
    let plain = Recipe {
        map: vec![(GRIN, 'ง')],
        weight: Some(900),
        ..Recipe::default()
    };
    let colour = newer(Node::glyph('ข', Node::Solid(BLUE)));
    let fonts = install(&[plain.build(), colour.build()]);
    let mut heavy = title(&format!("Go {GRIN}"));
    heavy.font_weight = 900.0;
    assert!(count(&draw(&fonts.fonts, &heavy), blue) > 200);
}

/// A clip box cuts the picture where it says.
#[test]
fn a_clip_box_cuts_the_picture() {
    let edge = bounds('ข');
    let whole = install(&[newer(Node::glyph('ข', Node::Solid(RED))).build()]);
    let full = extent(&draw(&whole.fonts, &title(&GRIN.to_string())), red).expect("drawn");
    let middle = (edge.x_min + edge.x_max) / 2;
    let cut = Recipe {
        clip: Some(('ก', [edge.x_min - 10, -1000, middle, 2000])),
        ..newer(Node::glyph('ข', Node::Solid(RED)))
    };
    let cut = install(&[cut.build()]);
    let half = extent(&draw(&cut.fonts, &title(&GRIN.to_string())), red).expect("drawn");
    let ratio = (half[2] - half[0]) as f32 / (full[2] - full[0]) as f32;
    assert!((0.35..=0.65).contains(&ratio), "{ratio}: {full:?} {half:?}");
    assert_eq!(half[0], full[0], "cut on the right");
}

/// A transform in the layers scales what is under it.
#[test]
fn a_transform_scales_the_picture() {
    let plain = install(&[newer(Node::glyph('ข', Node::Solid(RED))).build()]);
    let scaled = install(&[newer(Node::Scale(
        0.5,
        Box::new(Node::glyph('ข', Node::Solid(RED))),
    ))
    .build()]);
    let one = extent(&draw(&plain.fonts, &title(&GRIN.to_string())), red).expect("drawn");
    let half = extent(&draw(&scaled.fonts, &title(&GRIN.to_string())), red).expect("drawn");
    let ratio = (half[3] - half[1]) as f32 / (one[3] - one[1]) as f32;
    assert!((0.4..=0.6).contains(&ratio), "{ratio}");
}

/// Groups go down by their blend: kept under, the backdrop alone shows;
/// laid over, the source covers it.
#[test]
fn composites_follow_their_mode() {
    let of = |mode: u8| {
        newer(Node::Composite(
            Box::new(Node::glyph('ข', Node::Solid(GREEN))),
            mode,
            Box::new(Node::glyph('ข', Node::Solid(RED))),
        ))
    };
    const DESTINATION: u8 = 2;
    const SOURCE_OVER: u8 = 3;
    let under = install(&[of(DESTINATION).build()]);
    let under = draw(&under.fonts, &title(&GRIN.to_string()));
    assert!(count(&under, red) > 200);
    assert_eq!(count(&under, green), 0);
    let over = install(&[of(SOURCE_OVER).build()]);
    let over = draw(&over.fonts, &title(&GRIN.to_string()));
    assert!(count(&over, green) > 200);
    assert_eq!(count(&over, red), 0);
}

/// A layer in the text's own colour takes the fill, each fill painted once
/// however often the title is.
#[test]
fn newer_layers_take_the_text_colour() {
    let fonts = install(&[newer(Node::glyph('ข', Node::Solid(0xFFFF))).build()]);
    let mut style = title(&GRIN.to_string());
    style.color = "#ff00ff".to_owned();
    for _ in 0..3 {
        assert!(count(&draw(&fonts.fonts, &style), magenta) > 200);
    }
    style.color = "#00ff00".to_owned();
    for _ in 0..3 {
        assert!(count(&draw(&fonts.fonts, &style), green) > 200);
    }
    assert_eq!(fonts.fonts.pictures.paints(), 2, "one picture per colour");
}

/// A sweep turns counter-clockwise from its start angle, and a radial
/// gradient runs out from its centre.
#[test]
fn sweeps_and_radial_gradients_run_their_way() {
    let square = [0, 0, 800, 800];
    // 0° to 90°, stored a half turn short.
    let sweep = Recipe {
        clip: Some(('ก', square)),
        ..newer(Node::Sweep(
            [400, 400],
            -1.0,
            -0.5,
            vec![(0.0, RED), (1.0, BLUE)],
        ))
    };
    let fonts = install(&[sweep.build()]);
    let frame = draw(&fonts.fonts, &title(&GRIN.to_string()));
    let [l, t, r, b] = extent(&frame, solid).expect("drawn");
    let (cx, cy, reach) = ((l + r) / 2, (t + b) / 2, (r - l) * 3 / 8);
    let tenth = reach / 5;
    assert!(
        reddish(at(&frame, cx + reach, cy - tenth)),
        "just above the x axis: the start"
    );
    assert!(
        bluish(at(&frame, cx + tenth, cy - reach)),
        "just right of the y axis: the end"
    );
    // A quarter turn, not the whole circle: past its end, the end colour.
    assert!(bluish(at(&frame, cx - reach, cy)), "the far side");
    assert!(bluish(at(&frame, cx + tenth, cy + reach)), "below");

    let radial = Recipe {
        clip: Some(('ก', square)),
        ..newer(Node::Radial(
            [400, 400, 0, 400, 400, 400],
            vec![(0.0, RED), (1.0, BLUE)],
        ))
    };
    let fonts = install(&[radial.build()]);
    let frame = draw(&fonts.fonts, &title(&GRIN.to_string()));
    let [l, t, r, b] = extent(&frame, solid).expect("drawn");
    assert!(reddish(at(&frame, (l + r) / 2, (t + b) / 2)), "the centre");
    assert!(bluish(at(&frame, l + 1, t + 1)), "a corner");
}

/// Newer layers that put colour nowhere leave the glyph to its outline in
/// the fill.
#[test]
fn newer_layers_that_show_nothing_fall_back_to_the_outline() {
    let fonts = install(&[newer(Node::glyph(' ', Node::Solid(RED))).build()]);
    let mut style = title(&GRIN.to_string());
    style.color = "#ff00ff".to_owned();
    let frame = draw(&fonts.fonts, &style);
    assert!(count(&frame, magenta) > 100);
    assert_eq!(count(&frame, red), 0);
}

/// Paints garbled every which way - formats, offsets, colours, counts -
/// draw something or nothing, and never bring the title down. (The bytes
/// before the paints are left alone: the font library checks the table's
/// own counts with debug assertions of its own, before anything here
/// runs.)
#[test]
fn garbled_colour_tables_never_panic() {
    let good = Recipe {
        clip: Some(('ก', [0, 0, 800, 800])),
        ..newer(Node::Composite(
            Box::new(Node::Scale(
                0.5,
                Box::new(Node::glyph(
                    'ข',
                    Node::Linear([0, 0, 500, 0, 0, 500], vec![(0.0, RED), (1.0, BLUE)]),
                )),
            )),
            12,
            Box::new(Node::glyph(
                'ค',
                Node::Sweep([300, 300], -1.0, 1.0, vec![(0.2, GREEN), (0.8, RED)]),
            )),
        ))
    };
    let colr = good.colr();
    // The header, the base glyph list's count and its one record.
    let paints = 34 + 4 + 6;
    let mut seed = 0x9E37_79B9_u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for _ in 0..40 {
        let mut garbled = colr.clone();
        for _ in 0..4 {
            let at = paints + next() as usize % (garbled.len() - paints - 20);
            garbled[at] = next() as u8;
        }
        let mut tables = tables_of(&good.build());
        tables.insert(*b"COLR", garbled);
        let fonts = install(&[font_of(&tables)]);
        draw(&fonts.fonts, &title(&format!("a{GRIN}b")));
    }
}

// ── which face, by what the words ask ──

/// A character the title's face has, followed by the selector that asks
/// for a picture, is drawn by a colour face that has one; without it, it
/// stays a letter of the title's face.
#[test]
fn a_picture_asked_for_is_drawn_in_colour() {
    let bundled = ttf_parser::Face::parse(BUNDLED[0], 0).expect("the bundled face parses");
    assert!(bundled.glyph_index('©').is_some(), "the title's face has ©");
    let face = Recipe {
        map: vec![('©', 'ก')],
        sbix: vec![(64, 'ก', png(64, [0, 255, 0, 255]))],
        ..Recipe::default()
    };
    let fonts = install(&[face.build()]);
    assert!(count(&draw(&fonts.fonts, &title("©\u{FE0F}")), green) > 1000);
    assert_eq!(count(&draw(&fonts.fonts, &title("©")), green), 0);
}

/// An emoji followed by the selector that asks for a letter is drawn by a
/// plain face that has it, where one does; without it, in colour.
#[test]
fn a_letter_asked_for_is_drawn_as_one() {
    let plain = Recipe {
        map: vec![(GRIN, 'ง')],
        ..Recipe::default()
    };
    let fonts = install(&[plain.build(), square(64, 64).build()]);
    assert!(count(&draw(&fonts.fonts, &title(&GRIN.to_string())), green) > 1000);
    let letter = draw(&fonts.fonts, &title(&format!("{GRIN}\u{FE0E}")));
    assert_eq!(count(&letter, coloured), 0);
    assert!(count(&letter, white) > 100, "the plain face's outline");
}

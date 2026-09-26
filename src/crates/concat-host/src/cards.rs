// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Effect cards: every picture package's still, drawn by the package.
//!
//! A card is one reference picture through a package at its defaults,
//! drawn by the same shader and the same compositor a clip's frames go
//! through, so a card shows what the effect does and cannot drift from it.
//! An effect or a look treats the picture; a text effect treats a title set
//! over it; a transition cuts from the picture to a mirrored, hue-turned
//! copy of it, partway through. A package with no shader - someone's own
//! format 1 package - is drawn through its FFmpeg chain. Whatever a card
//! leaves transparent - a mask, a key - shows a checkerboard. A package
//! whose defaults show little on a card says what its card shows in its
//! manifest's `[card]` table: knob values, the moment, the point of the
//! cut, a screen behind the picture for a key to take out.
//!
//! Cards are drawn once and kept as JPEGs in a folder, each named after its
//! package and a fingerprint of everything it was drawn from: the shader,
//! the knobs' defaults, the table, and how cards are drawn. A package whose
//! shader or defaults change has a new name and so a new card, and the old
//! one is swept away ([`prune`]). The window reads what is there as it opens
//! and draws the rest on a thread of its own ([`Painter`]).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use concat_core::frame::Frame;
use concat_core::shader::{RevealMap, ShaderPass, TransitionPass};
use concat_core::time::Rational;
use concat_effects::{At, Catalogue, Package};
use concat_media::{DecodeOptions, Decoder, FrameSource};
use concat_render::{Compositor, FramePlan, PlannedLayer, WgpuCompositor, detached_clip};

/// A card's width in pixels: 16:9, and twice the widest a card is shown,
/// for a high-density screen.
pub const WIDTH: u32 = 480;

/// A card's height in pixels.
pub const HEIGHT: u32 = 270;

/// The picture every card is drawn from: a person in a bright, colourful
/// room, so a look has skin, saturated colour, shadow and highlight to
/// work on.
pub const STILL: &[u8] = include_bytes!("../assets/card-still.jpg");

/// Bumped whenever the way a card is drawn changes - the still, the moment,
/// the title, the size, the encoding - so every kept card is drawn again.
const EDITION: u64 = 1;

/// The instant a card shows, in seconds into its clip: late enough that a
/// reveal is under way and a pulse is lit, early enough that a one-shot
/// look has not finished.
const MOMENT: f64 = 0.9;

/// How far through its cut a transition's card shows: both pictures in
/// view, the incoming one not yet past the middle.
const CUT_AT: f32 = 0.4;

/// The words a text effect's card sets over the still.
const TITLE: &str = "Big Title";

/// The quality cards are written at: FFmpeg's JPEG quantiser, 2 the best.
const QUALITY: u8 = 3;

/// What a card draws.
#[derive(Clone, Debug)]
enum Drawing {
    /// The still through a pass, over a checkerboard; before a screen of
    /// the colour given, for a key.
    Picture {
        pass: ShaderPass,
        screen: Option<[u8; 3]>,
    },
    /// A title over the still, the pass on the title.
    Title(ShaderPass),
    /// A cut from the still to its turned copy.
    Cut(TransitionPass),
    /// The still through an FFmpeg chain.
    Chain(String),
}

/// One package's card: where it is kept, and what it draws.
#[derive(Clone, Debug)]
pub struct Card {
    /// The package's id.
    pub id: String,
    /// The JPEG the card is kept in; see [`Card::is_drawn`].
    pub path: PathBuf,
    drawing: Drawing,
    /// Seconds into the clip the card shows.
    moment: f64,
}

impl Card {
    /// The card `package` has, kept under `dir`, or None for a package
    /// with nothing to draw: a sound, or a package with neither a shader
    /// nor a chain.
    pub fn of(catalogue: &Catalogue, package: &Package, dir: &Path) -> Option<Card> {
        if !package.kind().is_visual() {
            return None;
        }
        let settings = package.manifest.card.clone().unwrap_or_default();
        let mut set = package.params_at(At::Default);
        set.extend(settings.params.clone());
        let moment = settings.moment.unwrap_or(MOMENT);
        let drawing = if package.transition().is_some() {
            Drawing::Cut(catalogue.transition_pass(
                package.id(),
                &settings.params,
                settings.progress.unwrap_or(f64::from(CUT_AT)),
            )?)
        } else if let Some(pass) = package.pass(&set, None) {
            if package.category().eq_ignore_ascii_case("Text") {
                Drawing::Title(pass)
            } else {
                Drawing::Picture {
                    pass,
                    screen: settings.screen_rgb(),
                }
            }
        } else {
            Drawing::Chain(package.ffmpeg_fragment(&set, 0).ok().flatten()?)
        };
        let name = format!(
            "{}-{:016x}.jpg",
            package.id(),
            fingerprint(&drawing, moment)
        );
        Some(Card {
            id: package.id().to_owned(),
            path: dir.join(name),
            drawing,
            moment,
        })
    }

    /// Whether the card is drawn and kept.
    pub fn is_drawn(&self) -> bool {
        self.path.is_file()
    }
}

/// Every card the catalogue's picture packages have, kept under `dir`.
pub fn cards(catalogue: &Catalogue, dir: &Path) -> Vec<Card> {
    catalogue
        .packages()
        .filter_map(|package| Card::of(catalogue, package, dir))
        .collect()
}

/// Removes every kept card under `dir` that is not one of `cards`: an
/// older drawing of a package that has changed, or the card of one that
/// is gone. Anything in the folder that is not named like a card is left.
pub fn prune(dir: &Path, cards: &[Card]) {
    let keep: HashSet<&Path> = cards.iter().map(|card| card.path.as_path()).collect();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        let named = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".jpg"))
            .and_then(|stem| stem.rsplit_once('-'))
            .is_some_and(|(_, print)| {
                print.len() == 16 && print.chars().all(|c| c.is_ascii_hexdigit())
            });
        if named && !keep.contains(path.as_path()) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// FNV-1a over everything a card is drawn from, so its name is the same on
/// every run and every build: a kept card is found again, and a changed
/// package is not mistaken for its old self.
fn fingerprint(drawing: &Drawing, moment: f64) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        // A separator, so two fields cannot run into one another.
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    feed(&EDITION.to_le_bytes());
    feed(&WIDTH.to_le_bytes());
    feed(&HEIGHT.to_le_bytes());
    feed(&moment.to_le_bytes());
    let pass = |feed: &mut dyn FnMut(&[u8]), pass: &ShaderPass| {
        feed(pass.key.as_bytes());
        feed(&pass.params);
        feed(&pass.intensity.to_le_bytes());
        feed(&pass.lut.as_ref().map_or(0, |lut| lut.id).to_le_bytes());
    };
    match drawing {
        Drawing::Picture {
            pass: drawn,
            screen,
        } => {
            feed(b"picture");
            feed(&screen.map_or([0; 4], |[r, g, b]| [1, r, g, b]));
            pass(&mut feed, drawn);
        }
        Drawing::Title(drawn) => {
            feed(b"title");
            feed(TITLE.as_bytes());
            pass(&mut feed, drawn);
        }
        Drawing::Cut(cut) => {
            feed(b"cut");
            feed(cut.key.as_bytes());
            feed(&cut.params);
            feed(&cut.progress.to_le_bytes());
            feed(&cut.lut.as_ref().map_or(0, |lut| lut.id).to_le_bytes());
        }
        Drawing::Chain(chain) => {
            feed(b"chain");
            feed(chain.as_bytes());
        }
    }
    hash
}

/// What draws cards: a compositor, and the pictures every card starts from,
/// made once - the still at the card's size, its turned copy, the
/// checkerboard, the title.
pub struct Painter {
    compositor: WgpuCompositor,
    still: Arc<Frame>,
    turned: Arc<Frame>,
    checker: Arc<Frame>,
    title: Option<(Arc<Frame>, Arc<RevealMap>)>,
}

impl Painter {
    /// A painter on `compositor`, drawing from the picture at `still`
    /// scaled to `width` by `height`.
    pub fn new(
        compositor: WgpuCompositor,
        still: &Path,
        width: u32,
        height: u32,
    ) -> Result<Painter, String> {
        let mut decoder = Decoder::open(still, &DecodeOptions::default().scaled_to(width, height))
            .map_err(|error| error.to_string())?;
        let still = decoder
            .next_frame()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("{} holds no picture", still.display()))?;
        let turned = turned(&still);
        let checker = checker(still.width(), still.height());
        Ok(Painter {
            compositor,
            still: Arc::new(still),
            turned: Arc::new(turned),
            checker: Arc::new(checker),
            title: None,
        })
    }

    /// Draws `card` and writes it to its path, whole or not at all.
    pub fn draw(&mut self, card: &Card) -> Result<(), String> {
        let frame = self.frame(card)?;
        write_card(&frame, &card.path)
    }

    /// Draws `card` and writes it to `path` instead of its own.
    pub fn draw_to(&mut self, card: &Card, path: &Path) -> Result<(), String> {
        let frame = self.frame(card)?;
        write_card(&frame, path)
    }

    /// `card` drawn, as an opaque frame the still's size.
    pub fn frame(&mut self, card: &Card) -> Result<Frame, String> {
        let (width, height) = (self.still.width(), self.still.height());
        let moment = Rational::approximate(card.moment).ok_or("the moment is not a time")?;
        let frame = match &card.drawing {
            Drawing::Picture { pass, screen } => {
                let subject = match screen {
                    Some(colour) => Arc::new(screened(&self.still, *colour)),
                    None => Arc::clone(&self.still),
                };
                let ground = PlannedLayer::picture(detached_clip(), Arc::clone(&self.checker));
                let mut picture = PlannedLayer::picture(detached_clip(), subject);
                picture.track = 1;
                picture.effects = vec![pass.clone()];
                self.compositor.render(&FramePlan {
                    time: moment,
                    layers: vec![ground, picture],
                    ..FramePlan::empty(width, height)
                })
            }
            Drawing::Title(pass) => {
                let (title, reveal) = self.title(width, height)?;
                let ground = PlannedLayer::picture(detached_clip(), Arc::clone(&self.still));
                let mut words = PlannedLayer::picture(detached_clip(), title);
                words.track = 1;
                let mut pass = pass.clone();
                pass.reveal_map = Some(reveal);
                words.effects = vec![pass];
                self.compositor.render(&FramePlan {
                    time: moment,
                    layers: vec![ground, words],
                    ..FramePlan::empty(width, height)
                })
            }
            Drawing::Cut(cut) => self
                .compositor
                .combine(
                    width,
                    height,
                    card.moment as f32,
                    &self.still,
                    &self.turned,
                    cut,
                )
                .ok_or("the transition's shader would not run")?,
            Drawing::Chain(chain) => {
                concat_media::treat(&self.still, chain).map_err(|error| error.to_string())?
            }
        };
        if self.compositor.lost() {
            return Err("the GPU device was lost".to_owned());
        }
        Ok(frame)
    }

    /// The title a text effect's card treats, set once: white words with a
    /// shadow, centred, over nothing, and the order its words reveal in.
    fn title(&mut self, width: u32, height: u32) -> Result<(Arc<Frame>, Arc<RevealMap>), String> {
        if let Some((frame, reveal)) = &self.title {
            return Ok((Arc::clone(frame), Arc::clone(reveal)));
        }
        let style = concat_text::TitleStyle {
            content: TITLE.to_owned(),
            font_family: concat_text::BUNDLED_FAMILY.to_owned(),
            font_size: 0.26,
            font_weight: 800.0,
            italic: false,
            color: "#ffffff".to_owned(),
            align: concat_text::Align::Center,
            stroke_width: 0.0,
            stroke_color: "#000000".to_owned(),
            shadow: true,
            background: String::new(),
            background_radius: 0.0,
            background_padding_x: 0.0,
            background_padding_y: 0.0,
            line_height: 1.1,
            tracking: 0.0,
            max_width: 0.0,
            max_height: 0.0,
        };
        let fonts = concat_text::Fonts::new();
        let rendered = concat_text::render_frame(&fonts, &style, width, height)
            .map_err(|error| error.to_string())?;
        let rects: Vec<(i32, i32, u32, u32)> = rendered
            .words
            .iter()
            .map(|word| (word.x, word.y, word.width, word.height))
            .collect();
        let reveal = Arc::new(RevealMap::from_rects(width, height, &rects));
        let frame = Arc::new(
            Frame::from_rgba(rendered.width, rendered.height, rendered.rgba)
                .ok_or("the title came back the wrong size")?,
        );
        self.title = Some((Arc::clone(&frame), Arc::clone(&reveal)));
        Ok((frame, reveal))
    }
}

/// The still a transition cuts to: mirrored, and every hue turned half way
/// round, so the two pictures of a cut can be told apart anywhere on it.
fn turned(still: &Frame) -> Frame {
    let (width, height) = (still.width(), still.height());
    let mut out = still.clone();
    for y in 0..height {
        for x in 0..width {
            let [r, g, b, a] = still.pixel(width - 1 - x, y).unwrap_or([0, 0, 0, 255]);
            let [r, g, b] = [r, g, b].map(|c| f32::from(c) / 255.0);
            // Half a turn in YIQ: the two chroma axes negated, the
            // brightness kept.
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            let [r, g, b] = [2.0 * luma - r, 2.0 * luma - g, 2.0 * luma - b]
                .map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            out.set_pixel(x, y, [r, g, b, a]);
        }
    }
    out
}

/// Draws `cards` on `compositor` - the window's device, else whatever
/// adapter is to be had - handing their ids to `report` a few at a time as
/// they land, so a shelf fills as it goes, and a last time with `done`. A
/// card that fails is logged and skipped: its package shows no picture.
pub fn draw_all(
    dir: &Path,
    cards: &[Card],
    compositor: Option<WgpuCompositor>,
    mut report: impl FnMut(Vec<String>, bool),
) {
    const BATCH: usize = 12;
    let painter = compositor
        .or_else(WgpuCompositor::new)
        .ok_or_else(|| "no GPU or software renderer is available to draw with".to_owned())
        .and_then(|compositor| Painter::new(compositor, &still_in(dir)?, WIDTH, HEIGHT));
    let mut painter = match painter {
        Ok(painter) => painter,
        Err(error) => {
            log::warn!("cards: {error}");
            report(Vec::new(), true);
            return;
        }
    };
    let mut batch = Vec::new();
    for card in cards {
        match painter.draw(card) {
            Ok(()) => batch.push(card.id.clone()),
            Err(error) => log::warn!("card {}: {error}", card.id),
        }
        if batch.len() >= BATCH {
            report(std::mem::take(&mut batch), false);
        }
    }
    report(batch, true);
}

/// What shows through where a card is transparent: grey squares a
/// twentieth of the height, the way every editor says "nothing here".
fn checker(width: u32, height: u32) -> Frame {
    let side = (height / 20).max(1);
    let mut frame = Frame::transparent(width, height);
    for y in 0..height {
        for x in 0..width {
            let light = ((x / side) + (y / side)).is_multiple_of(2);
            let level = if light { 0x5a } else { 0x44 };
            frame.set_pixel(x, y, [level, level, level, 255]);
        }
    }
    frame
}

/// The still before a screen of `colour`: everything left of the person
/// in the middle, and right of him, painted that colour, as a key's card
/// needs something to take out.
fn screened(still: &Frame, [r, g, b]: [u8; 3]) -> Frame {
    let (width, height) = (still.width(), still.height());
    let (left, right) = (width * 33 / 100, width * 80 / 100);
    let mut out = still.clone();
    for y in 0..height {
        for x in (0..left).chain(right..width) {
            out.set_pixel(x, y, [r, g, b, 255]);
        }
    }
    out
}

/// `frame` written to `path` as a JPEG, through a file beside it that is
/// renamed into place, so a reader never meets half a card.
fn write_card(frame: &Frame, path: &Path) -> Result<(), String> {
    let bytes = concat_media::jpeg(frame, QUALITY).map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let part = path.with_extension("jpg.part");
    std::fs::write(&part, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&part, path).map_err(|error| error.to_string())
}

/// The still, written into `dir` for a decoder to read, and its path. It is
/// rewritten when it differs, so a build with a new still draws from it.
pub fn still_in(dir: &Path) -> Result<PathBuf, String> {
    let path = dir.join("still.jpg");
    if std::fs::read(&path).ok().as_deref() != Some(STILL) {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        std::fs::write(&path, STILL).map_err(|error| error.to_string())?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("concat-cards-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        dir
    }

    /// Every picture package has a card and every sound has none, each
    /// under a name of its own.
    #[test]
    fn every_picture_package_has_a_card_of_its_own() {
        let catalogue = Catalogue::builtin();
        let dir = Path::new("cards");
        let cards = cards(catalogue, dir);
        let pictures = catalogue
            .packages()
            .filter(|package| package.kind().is_visual())
            .count();
        assert_eq!(cards.len(), pictures);
        let names: HashSet<&Path> = cards.iter().map(|card| card.path.as_path()).collect();
        assert_eq!(names.len(), cards.len(), "two cards share a name");
        for card in &cards {
            let package = catalogue.get(&card.id).expect("a package");
            assert!(package.kind().is_visual(), "{}", card.id);
            assert!(card.path.starts_with(dir), "{}", card.path.display());
        }
    }

    /// A card's name follows what it is drawn from: the same package gives
    /// the same name, another shader or another default another.
    #[test]
    fn a_card_is_named_after_what_it_draws() {
        let catalogue = Catalogue::builtin();
        let dir = Path::new("cards");
        let manifest = |default: f64| {
            format!(
                "format = 2\n[effect]\nid = \"a.gain\"\nname = \"Gain\"\nkind = \"effect\"\n\
                 [[param]]\nkey = \"gain\"\nlabel = \"Gain\"\nmax = 4\ndefault = {default}\n\
                 [wgsl]\nentry = \"effect.wgsl\"\n"
            )
        };
        let shader = |k: f64| {
            format!(
                "struct Params {{ gain: f32 }}\nfn effect(uv: vec2<f32>) -> vec4<f32> {{ let c = sample(uv); return vec4<f32>(c.rgb * params.gain * {k:.1}, c.a); }}"
            )
        };
        let card = |default: f64, k: f64| {
            let package =
                Package::from_sources(&manifest(default), None, Some(&shader(k))).expect("loads");
            Card::of(catalogue, &package, dir).expect("a card").path
        };
        assert_eq!(card(2.0, 1.0), card(2.0, 1.0));
        assert_ne!(card(2.0, 1.0), card(3.0, 1.0), "another default");
        assert_ne!(card(2.0, 1.0), card(2.0, 2.0), "another shader");
        assert!(
            card(2.0, 1.0)
                .to_string_lossy()
                .starts_with(&*dir.join("a.gain-").to_string_lossy())
        );
    }

    /// Pruning sweeps away a card no package draws any more and leaves the
    /// cards that are current, the still and anything else in the folder.
    #[test]
    fn prune_sweeps_only_stale_cards() {
        let dir = scratch("prune");
        let catalogue = Catalogue::builtin();
        let current: Vec<Card> = cards(catalogue, &dir).into_iter().take(2).collect();
        for card in &current {
            std::fs::write(&card.path, b"card").expect("write");
        }
        let stale = dir.join("concat.glow-0123456789abcdef.jpg");
        let other = dir.join("notes.jpg");
        let still = still_in(&dir).expect("the still");
        std::fs::write(&stale, b"old").expect("write");
        std::fs::write(&other, b"mine").expect("write");
        prune(&dir, &current);
        assert!(!stale.exists(), "a stale card");
        assert!(other.exists() && still.exists(), "not cards");
        assert!(current.iter().all(Card::is_drawn), "current cards");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every card draws on the GPU, and every one shows its package at
    /// work: the picture it was drawn from - the still, or the still
    /// before its screen - comes out changed, where the FFmpeg-drawn cards
    /// of old came out all but untouched for a mask, a key or a mirror. Set
    /// `CONCAT_CARDS_DIR` to keep the cards for a look.
    #[test]
    fn every_card_draws_and_shows_its_package_at_work() {
        let Some(compositor) = WgpuCompositor::new() else {
            eprintln!("no usable GPU adapter; skipping");
            return;
        };
        let dir = std::env::var_os("CONCAT_CARDS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| scratch("draw"));
        let still = still_in(&dir).expect("the still");
        let mut painter = Painter::new(compositor, &still, WIDTH, HEIGHT).expect("a painter");
        let mut idle = Vec::new();
        let mut failed = Vec::new();
        let cards = cards(Catalogue::builtin(), &dir);
        for card in &cards {
            let frame = match painter.frame(card) {
                Ok(frame) => frame,
                Err(error) => {
                    failed.push(format!("{}: {error}", card.id));
                    continue;
                }
            };
            assert_eq!((frame.width(), frame.height()), (WIDTH, HEIGHT));
            let untreated = match &card.drawing {
                Drawing::Picture {
                    screen: Some(colour),
                    ..
                } => screened(&painter.still, *colour),
                _ => (*painter.still).clone(),
            };
            let change = frame
                .pixels()
                .iter()
                .zip(untreated.pixels())
                .map(|(a, b)| f64::from(a.abs_diff(*b)))
                .sum::<f64>()
                / frame.pixels().len() as f64;
            eprintln!("{:40} {change:6.2}", card.id);
            if change < 0.02 {
                idle.push(format!(
                    "{}: changes the still by {change:.2} a channel",
                    card.id
                ));
            }
            write_card(&frame, &card.path).expect("writes");
            assert!(card.is_drawn());
        }
        assert!(failed.is_empty(), "\n{}", failed.join("\n"));
        assert!(idle.is_empty(), "\n{}", idle.join("\n"));
        if std::env::var_os("CONCAT_CARDS_DIR").is_none() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

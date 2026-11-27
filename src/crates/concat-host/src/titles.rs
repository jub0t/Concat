// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Titles and shapes, rendered and remembered.
//!
//! The flattener leaves text and shape clips out on purpose: the compositor
//! only knows pictures. This is where a title or a shape becomes one. Each
//! clip's style is painted by `concat-text` onto a frame-sized transparent
//! PNG in the app's data directory, and the clip rejoins the flattened list
//! as an image clip pointing at that file, carrying the clip's timing,
//! transform and opacity. The monitor, playback prefetch and the exporter
//! then treat it as any other still.
//!
//! The file is keyed by everything that changes the pixels - the style, the
//! frame size, the fonts the project carries - so a title that has not
//! changed is never painted twice, across sessions included. What a title
//! does *not* key on is where it sits or when it plays: moving one is free.
//! A shape is a title with no words, as far as this file is concerned.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use concat_core::frame::Frame;
use concat_core::shader::RevealMap;
use concat_media::SparseStill;

use concat_doc::model::{
    Color, Content, Item, ItemId, LineHead, Model, ShapeKind, ShapeStyle, StrokeAlign, StrokeDash,
    TextAlign, Title,
};
use concat_export::{ClipKind, ExportClip};
use concat_text::{Align, Figure, Fonts, TitleRun, TitleStyle, WordRect};

use crate::dirs::AppDirs;

/// One title or shape, ready for the compositor.
#[derive(Clone)]
pub struct TitleClip {
    /// The text or shape clip this was painted from.
    pub clip_id: ItemId,
    /// The image clip that stands in for it.
    pub clip: ExportClip,
    /// The painted block's size in frame pixels, for an outline on a monitor.
    pub block: (u32, u32),
    /// Where that block's centre is, as an offset in frame pixels from the
    /// clip's own centre: zero for a centred title, half a block to the
    /// right for a left-aligned one, whose block starts at the clip's
    /// position. See `concat_text::Align`.
    pub offset: (i32, i32),
    /// The pixels, when the title was painted in memory rather than to
    /// disk: what the monitor is to hold under `clip.path`, a name no file
    /// has, as the part of the canvas with ink. See [`Titles::clips_live`].
    pub held: Option<Arc<SparseStill>>,
}

/// What is painted: a title's words or a shape's figure. One painter for
/// both, since they are cached, keyed and handed on the same way.
#[derive(Clone, Copy)]
enum Painting<'a> {
    Title(&'a Title),
    Shape(&'a ShapeStyle),
}

/// What one render left behind.
#[derive(Clone, Debug)]
struct Art {
    block: (u32, u32),
    offset: (i32, i32),
    /// The canvas the painting is on: the frame, or larger by the same
    /// margin either side where the title's ink runs past the frame's
    /// edges. See `concat_text::Rendered::overscan`.
    canvas: (u32, u32),
    /// The title's per-word reveal order, baked once here rather than on
    /// every clip build - see `concat_core::RevealMap`.
    reveal: Arc<RevealMap>,
}

/// The title painter and its cache.
pub struct Titles {
    dir: PathBuf,
    /// The system's faces plus the project's files, loaded once each.
    fonts: Mutex<Option<Fonts>>,
    loaded_files: Mutex<HashSet<String>>,
    /// What is known to be on disk, by key.
    memo: Mutex<HashMap<u64, Art>>,
    /// Titles painted live, by key: every title on the timeline, and the
    /// most recently asked for of the rest, so a width dragged back and
    /// forth over the same values paints each once.
    live: Mutex<LivePaintings>,
    /// The live painting last shown of each clip, by its id: what stands
    /// in while its new look paints. See [`Titles::ready_live`].
    shown: Mutex<HashMap<ItemId, u64>>,
}

/// The live paintings kept, in bytes of pixels, beyond the ones the
/// timeline is showing; see [`LivePaintings`]. Each is held as the part of
/// its canvas with ink - a caption's line of words, a few hundred
/// kilobytes at 1080p, where the whole canvas was 8 MB - so this keeps
/// hundreds.
const LIVE_BYTES: usize = 128 * 1024 * 1024;

/// The most the paintings of the titles on the timeline may hold between
/// them, in bytes of pixels: well over a thousand captions' words. Past it
/// the first painted are kept and the rest painted again each time; see
/// [`LivePaintings::keep`].
const LIVE_CEILING: usize = 512 * 1024 * 1024;

/// A `1 / part` share of a machine's `total` memory, never less than
/// `floor`: what the live paintings may hold on a machine that has more
/// to give. On 16 GB, half a gigabyte kept and two in all; on 64 GB, two
/// and eight - several hundred captions at the monitor's size, held as a
/// drag repaints the one being moved, where a fixed ceiling painted every
/// caption past it again at each step.
fn live_share(total: Option<u64>, part: u64, floor: usize) -> usize {
    total
        .and_then(|total| usize::try_from(total / part).ok())
        .map_or(floor, |share| share.max(floor))
}

/// One live painting, and the pass that last asked for it.
struct LivePainting {
    art: Art,
    held: Arc<SparseStill>,
    used: u64,
}

/// The titles painted live, by key, least recently used out first.
///
/// Every live ask paints every title on the timeline in turn, the same
/// order each time. Kept first in, first out - and a hit not counting as a
/// use - past 64 half-size portrait titles each one asked for was the one
/// thrown out the step before, so every title was painted again on every
/// change, and the editor lagged (#261). Now what the pass in hand has
/// asked for is never thrown out for the byte budget: the floor is every
/// title on the timeline, up to [`LIVE_CEILING`].
struct LivePaintings {
    kept: HashMap<u64, LivePainting>,
    /// Which pass over the timeline this is; see [`LivePaintings::begin`].
    pass: u64,
    /// Bytes of pixels held.
    bytes: usize,
    /// What the paintings past passes asked for may hold.
    budget: usize,
    /// What all of them may hold.
    ceiling: usize,
}

impl LivePaintings {
    fn new(budget: usize, ceiling: usize) -> Self {
        LivePaintings {
            kept: HashMap::new(),
            pass: 0,
            bytes: 0,
            budget,
            ceiling,
        }
    }

    /// Starts a pass over the timeline: what it asks for is held whatever
    /// the budget.
    fn begin(&mut self) {
        self.pass += 1;
    }

    /// The painting under `key`, now the most recently used.
    fn get(&mut self, key: u64) -> Option<(Art, Arc<SparseStill>)> {
        let pass = self.pass;
        self.kept.get_mut(&key).map(|painting| {
            painting.used = pass;
            (painting.art.clone(), Arc::clone(&painting.held))
        })
    }

    /// Whether a painting is kept under `key`, without counting it a use.
    fn has(&self, key: u64) -> bool {
        self.kept.contains_key(&key)
    }

    /// Keeps a painting under `key`. Over the budget, the least recently
    /// used go first, never one this pass has asked for. Over the ceiling
    /// with nothing else left, the one just painted is not kept: the ones
    /// painted before it stay for the next pass, rather than each pushing
    /// out the one it will want next. The newest is always kept alone.
    fn keep(&mut self, key: u64, art: Art, held: Arc<SparseStill>) {
        let size = held.bytes();
        let painting = LivePainting {
            art,
            held,
            used: self.pass,
        };
        if let Some(old) = self.kept.insert(key, painting) {
            self.bytes -= old.held.bytes();
        }
        self.bytes += size;
        while self.bytes > self.budget {
            let oldest = self
                .kept
                .iter()
                .filter(|(_, painting)| painting.used < self.pass)
                .min_by_key(|(_, painting)| painting.used)
                .map(|(key, _)| *key);
            let Some(oldest) = oldest else { break };
            self.drop_key(oldest);
        }
        if self.bytes > self.ceiling && self.kept.len() > 1 {
            self.drop_key(key);
        }
    }

    fn drop_key(&mut self, key: u64) {
        if let Some(gone) = self.kept.remove(&key) {
            self.bytes -= gone.held.bytes();
        }
    }
}

impl Titles {
    /// A painter that caches under the app's data directory.
    pub fn new(dirs: &AppDirs) -> Titles {
        let dir = dirs.data.join("titles");
        sweep(&dir, TITLES_KEPT);
        Titles {
            dir,
            fonts: Mutex::new(None),
            loaded_files: Mutex::new(HashSet::new()),
            memo: Mutex::new(HashMap::new()),
            live: Mutex::new(LivePaintings::new(
                live_share(crate::memory::total(), 32, LIVE_BYTES),
                live_share(crate::memory::total(), 8, LIVE_CEILING),
            )),
            shown: Mutex::new(HashMap::new()),
        }
    }

    /// Every text and shape clip on the active timeline, as an image clip
    /// each, for a `width` × `height` frame. One that fails to paint is left
    /// out and said once on stderr; the rest of the edit still renders.
    pub fn clips(&self, project: &Model, width: u32, height: u32) -> Vec<TitleClip> {
        self.clips_with(project, width, height, false, true, None)
            .unwrap_or_default()
            .0
    }

    /// [`Titles::clips`] for a job that can be stopped while its titles
    /// paint - an export: hundreds of captions are seconds of painting on a
    /// phone. None once `cancel` is set, the paintings finished so far kept
    /// for the next time they are asked for.
    pub fn clips_unless(
        &self,
        project: &Model,
        width: u32,
        height: u32,
        cancel: &AtomicBool,
    ) -> Option<Vec<TitleClip>> {
        self.clips_with(project, width, height, false, true, Some(cancel))
            .map(|(clips, _)| clips)
    }

    /// [`Titles::clips`] without painting: the titles painted already, and
    /// whether that is every one. For the window, which must never wait on
    /// a painter (hundreds of captions are seconds of painting on a phone)
    /// and paints what is missing on a thread of its own, drawing what is
    /// there meanwhile.
    pub fn ready(&self, project: &Model, width: u32, height: u32) -> (Vec<TitleClip>, bool) {
        self.clips_with(project, width, height, false, false, None)
            .unwrap_or_default()
    }

    /// [`Titles::clips`] for a monitor showing a change as it is made:
    /// every title is painted in memory at `width` by `height` - the
    /// monitor's own size, not the output's - and comes back with its
    /// pixels under a name no file has, for the monitor to hold. Nothing
    /// touches the disk, which is what makes a drag smooth: a PNG per
    /// pointer step, encoded, written and decoded again, was the lag.
    pub fn clips_live(&self, project: &Model, width: u32, height: u32) -> Vec<TitleClip> {
        self.clips_with(project, width, height, true, true, None)
            .unwrap_or_default()
            .0
    }

    /// [`Titles::clips_live`] without painting: the titles painted in
    /// memory already, and whether that is every one - what a window draws
    /// while [`Titles::paint_live_all`] paints the rest off its thread.
    pub fn ready_live(&self, project: &Model, width: u32, height: u32) -> (Vec<TitleClip>, bool) {
        self.clips_with(project, width, height, true, false, None)
            .unwrap_or_default()
    }

    /// For a change in flight: [`Titles::clips_live`] when `most` titles or
    /// fewer are not painted in memory yet - the one being changed, and so
    /// painted on the spot - and [`Titles::ready_live`] otherwise, leaving
    /// a timeline of captions not yet painted to [`Titles::paint_live_all`]
    /// rather than holding the window while it paints them all.
    pub fn clips_live_within(
        &self,
        project: &Model,
        width: u32,
        height: u32,
        most: usize,
    ) -> (Vec<TitleClip>, bool) {
        let missing = {
            let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            let mut seen = HashSet::new();
            keys_of(project, width, height)
                .into_iter()
                .flatten()
                .filter(|(key, _)| seen.insert(*key) && !live.has(*key))
                .count()
        };
        if missing <= most {
            (self.clips_live(project, width, height), true)
        } else {
            self.ready_live(project, width, height)
        }
    }

    /// Paints in memory every title on the timeline not painted there yet,
    /// on every core: the work [`Titles::ready_live`] leaves.
    pub fn paint_live_all(&self, project: &Model, width: u32, height: u32) {
        let keys = keys_of(project, width, height);
        self.paint_ahead(project, width, height, true, &keys, &|| false);
        // One alone is left to the pass by paint_ahead; paint it here.
        for (key, painting) in keys.into_iter().flatten() {
            if !self.live.lock().unwrap_or_else(|e| e.into_inner()).has(key) {
                let mut fonts = self.fonts.lock().unwrap_or_else(|e| e.into_inner());
                let fonts = self.fonts_for(project, &mut fonts);
                if let Err(error) = self.paint_live(fonts, key, painting, width, height) {
                    log::warn!("title: {error}");
                }
            }
        }
    }

    /// The titles as image clips, and whether every one is there: `paint`
    /// paints those that are not, and without it they are left out. None
    /// once `cancel` is set.
    fn clips_with(
        &self,
        project: &Model,
        width: u32,
        height: u32,
        live: bool,
        paint: bool,
        cancel: Option<&AtomicBool>,
    ) -> Option<(Vec<TitleClip>, bool)> {
        let stopped = || cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed));
        if live {
            self.live.lock().unwrap_or_else(|e| e.into_inner()).begin();
        }
        let keys = keys_of(project, width, height);
        if paint {
            self.paint_ahead(project, width, height, live, &keys, &stopped);
        }
        if stopped() {
            return None;
        }
        let mut out = Vec::new();
        let mut complete = true;
        for (placed, key) in placed(project).into_iter().zip(keys) {
            let Placed {
                id,
                item: clip,
                index,
                visible,
            } = placed;
            let Some((key, painting)) = key else {
                continue;
            };
            if stopped() {
                return None;
            }
            let key = if paint {
                key
            } else if live {
                let held = self.live.lock().unwrap_or_else(|e| e.into_inner());
                if held.has(key) {
                    key
                } else {
                    complete = false;
                    // Its new look not painted yet: the last one shown of
                    // this clip stands in until it is, so an edit to every
                    // caption at once leaves them showing, not gone.
                    let shown = self.shown.lock().unwrap_or_else(|e| e.into_inner());
                    match shown.get(&id).copied().filter(|last| held.has(*last)) {
                        Some(last) => last,
                        None => continue,
                    }
                }
            } else if self.kept_painting(key, width, height).is_none() {
                complete = false;
                continue;
            } else {
                key
            };
            if live {
                self.shown
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id, key);
            }
            let painted = if live {
                self.painted_live(project, key, painting, width, height)
                    .map(|(path, art, held)| (path, art, Some(held)))
            } else {
                self.painted(project, key, painting, width, height)
                    .map(|(path, art)| (path, art, None))
            };
            let base = picture_base(clip, index);
            let (path, art, held) = match painted {
                Ok(art) => art,
                Err(error) => {
                    log::warn!("title {id}: {error}");
                    continue;
                }
            };
            out.push(TitleClip {
                clip_id: id,
                clip: ExportClip {
                    path: path.to_string_lossy().into_owned(),
                    hidden: !visible,
                    muted: true,
                    volume: 0.0,
                    // A title's width and height keys go the way its
                    // constant pull does, below: never onto the glyphs.
                    animation: concat_export::flatten::keys(clip)
                        .into_iter()
                        .filter(|key| {
                            matches!(painting, Painting::Shape(_))
                                || !matches!(key.property.as_str(), "stretchX" | "stretchY")
                        })
                        .collect(),
                    // What a media clip carries in the flattening and a
                    // title or a shape would otherwise lose here, starting
                    // from blank: the Animate moves, the mask and the
                    // fades. Without them no In, Out or Loop played on any
                    // title, caption or shape.
                    motion: base.motion,
                    masks: concat_export::flatten::masks(clip),
                    fade_in: base.fade_in,
                    fade_out: base.fade_out,
                    flip_h: clip.picture.flip_h,
                    flip_v: clip.picture.flip_v,
                    blend: base.blend.clone(),
                    effects: concat_export::flatten::links(clip, &clip.effects),
                    // A keyed property's keys travel absolute, on a
                    // neutral base, as a media clip's do: the clip's own
                    // offset under keys put the title at the two added,
                    // away from the gizmo and often off the frame (#512).
                    scale: base.scale,
                    offset_x: base.offset_x,
                    offset_y: base.offset_y,
                    rotation: base.rotation,
                    // A title is never pulled along an axis: its box is
                    // sized by the style, and its glyphs keep their shape.
                    // 0.2.2's side grips wrote a stretch onto title clips
                    // to make a long caption fit, and honouring it here
                    // kept those captions squashed after the wrap arrived.
                    // https://github.com/jub0t/Concat/issues/119
                    // A shape is pulled: that is how a square becomes
                    // the rectangle that was wanted.
                    stretch_x: match painting {
                        Painting::Title(_) => 1.0,
                        Painting::Shape(_) => base.stretch_x,
                    },
                    stretch_y: match painting {
                        Painting::Title(_) => 1.0,
                        Painting::Shape(_) => base.stretch_y,
                    },
                    // The style's own opacity multiplies the clip's: a
                    // half-transparent title fades to half, not to solid.
                    opacity: match painting {
                        Painting::Title(text) => (base.opacity * text.opacity).clamp(0.0, 1.0),
                        Painting::Shape(_) => base.opacity.clamp(0.0, 1.0),
                    },
                    // The canvas, which is the frame's size unless the
                    // title's ink runs past the frame - a box pulled wider,
                    // a word too long for a line - when it is larger and
                    // drawn at one to one, so the ink is shown rather than
                    // cut where the frame ends.
                    media_width: Some(art.canvas.0),
                    media_height: Some(art.canvas.1),
                    overscan: overscan_of(art.canvas, width, height),
                    has_audio: Some(false),
                    // A shape has no words to reveal one at a time.
                    reveal_map: match painting {
                        Painting::Title(_) => Some(Arc::clone(&art.reveal)),
                        Painting::Shape(_) => None,
                    },
                    ..ExportClip::blank(ClipKind::Image, base.start, base.duration, index)
                },
                block: art.block,
                offset: art.offset,
                held,
            });
        }
        Some((out, complete))
    }

    /// One style at one size, painted in memory: the name the monitor is
    /// to hold the pixels under, the block, and the pixels.
    fn painted_live(
        &self,
        project: &Model,
        key: u64,
        painting: Painting<'_>,
        width: u32,
        height: u32,
    ) -> Result<(PathBuf, Art, Arc<SparseStill>), String> {
        if let Some(found) = self.live_painting(key) {
            return Ok(found);
        }
        let mut fonts = self.fonts.lock().unwrap_or_else(|e| e.into_inner());
        let fonts = self.fonts_for(project, &mut fonts);
        self.paint_live(fonts, key, painting, width, height)
    }

    /// The live painting kept under `key`, if there is one.
    fn live_painting(&self, key: u64) -> Option<(PathBuf, Art, Arc<SparseStill>)> {
        let path = PathBuf::from(format!("memory://titles/{key:016x}"));
        let (art, held) = self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)?;
        Some((path, art, held))
    }

    /// Paints one style in memory with `fonts`, and keeps it under `key`.
    fn paint_live(
        &self,
        fonts: &Fonts,
        key: u64,
        painting: Painting<'_>,
        width: u32,
        height: u32,
    ) -> Result<(PathBuf, Art, Arc<SparseStill>), String> {
        let path = PathBuf::from(format!("memory://titles/{key:016x}"));
        let rendered = match painting {
            Painting::Title(style) => {
                concat_text::render_frame(fonts, &title_style(style), width, height)
                    .map_err(|error| error.to_string())?
            }
            Painting::Shape(style) => {
                concat_text::render_shape_frame(&shape_style(style), width, height)
                    .map_err(|error| error.to_string())?
            }
        };
        let art = Art {
            block: (rendered.block_width, rendered.block_height),
            offset: (rendered.block_dx, rendered.block_dy),
            canvas: (rendered.width, rendered.height),
            reveal: reveal_of(&rendered.words, rendered.width, rendered.height),
        };
        let held = Arc::new(
            sparse_of(rendered.width, rendered.height, &rendered.rgba)
                .ok_or_else(|| "the painter returned a frame of the wrong size".to_owned())?,
        );
        self.live.lock().unwrap_or_else(|e| e.into_inner()).keep(
            key,
            art.clone(),
            Arc::clone(&held),
        );
        Ok((path, art, held))
    }

    /// The PNG for one style at one frame size, painted if it is not on disk
    /// yet, and the block it holds.
    fn painted(
        &self,
        project: &Model,
        key: u64,
        painting: Painting<'_>,
        width: u32,
        height: u32,
    ) -> Result<(PathBuf, Art), String> {
        if let Some(found) = self.kept_painting(key, width, height) {
            return Ok(found);
        }
        let mut fonts = self.fonts.lock().unwrap_or_else(|e| e.into_inner());
        let fonts = self.fonts_for(project, &mut fonts);
        self.paint_kept(fonts, key, painting, width, height)
    }

    /// The PNG kept under `key` and the block it holds, if it is on disk.
    fn kept_painting(&self, key: u64, width: u32, height: u32) -> Option<(PathBuf, Art)> {
        let png = self.dir.join(format!("{key:016x}.png"));
        let side = self.dir.join(format!("{key:016x}.json"));
        if let Some(art) = self
            .memo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            && png.is_file()
        {
            return Some((png, art.clone()));
        }
        // On disk from an earlier session: the sidecar says how big the
        // block was and where, and which words it held, which the PNG
        // alone cannot.
        if png.is_file()
            && let Some(art) = read_block(&side, width, height)
        {
            self.remember(key, art.clone());
            return Some((png, art));
        }
        None
    }

    /// Paints one style with `fonts` into a PNG kept under `key`.
    fn paint_kept(
        &self,
        fonts: &Fonts,
        key: u64,
        painting: Painting<'_>,
        width: u32,
        height: u32,
    ) -> Result<(PathBuf, Art), String> {
        let png = self.dir.join(format!("{key:016x}.png"));
        let side = self.dir.join(format!("{key:016x}.json"));
        let rendered = match painting {
            Painting::Title(style) => {
                concat_text::render(fonts, &title_style(style), width, height)
                    .map_err(|error| error.to_string())?
            }
            Painting::Shape(style) => concat_text::render_shape(&shape_style(style), width, height)
                .map_err(|error| error.to_string())?,
        };
        std::fs::create_dir_all(&self.dir).map_err(|error| error.to_string())?;
        std::fs::write(&png, &rendered.png).map_err(|error| error.to_string())?;
        let art = Art {
            block: (rendered.block_width, rendered.block_height),
            offset: (rendered.block_dx, rendered.block_dy),
            canvas: (rendered.width, rendered.height),
            reveal: reveal_of(&rendered.words, rendered.width, rendered.height),
        };
        let words: Vec<String> = rendered
            .words
            .iter()
            .map(|w| format!("[{},{},{},{}]", w.x, w.y, w.width, w.height))
            .collect();
        let _ = std::fs::write(
            &side,
            format!(
                "{{\"w\":{},\"h\":{},\"x\":{},\"y\":{},\"cw\":{},\"ch\":{},\"words\":[{}]}}",
                art.block.0,
                art.block.1,
                art.offset.0,
                art.offset.1,
                art.canvas.0,
                art.canvas.1,
                words.join(",")
            ),
        );
        self.remember(key, art.clone());
        Ok((png, art))
    }

    /// Paints every title and shape on the active timeline that is not
    /// painted yet, on every core at once, so the pass over the timeline
    /// after finds each one kept. A transcribed interview is hundreds of
    /// captions, each painted on its own; one after another they were
    /// seconds of a window that did not answer.
    ///
    /// The faces are shared by every thread: they are loaded first, with
    /// the project's own, and only read while the titles paint. Each
    /// thread stops at its next title once `stopped` says so.
    fn paint_ahead(
        &self,
        project: &Model,
        width: u32,
        height: u32,
        live: bool,
        keys: &[Option<(u64, Painting<'_>)>],
        stopped: &(dyn Fn() -> bool + Sync),
    ) {
        let mut seen = HashSet::new();
        let todo: Vec<(u64, Painting<'_>)> = keys
            .iter()
            .flatten()
            .copied()
            .filter(|(key, _)| seen.insert(*key))
            .filter(|(key, _)| {
                if live {
                    self.live_painting(*key).is_none()
                } else {
                    // Painted this session is enough to leave it to the
                    // pass, which looks for its file anyway: one look on
                    // the disk a title, not two.
                    !self
                        .memo
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .contains_key(key)
                        && self.kept_painting(*key, width, height).is_none()
                }
            })
            .collect();
        // One alone paints on this thread, in the pass, as it always did.
        if todo.len() < 2 {
            return;
        }
        let threads = std::thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .min(todo.len());
        let mut held = self.fonts.lock().unwrap_or_else(|e| e.into_inner());
        let fonts: &Fonts = self.fonts_for(project, &mut held);
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    loop {
                        if stopped() {
                            return;
                        }
                        let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&(key, painting)) = todo.get(at) else {
                            return;
                        };
                        // A failure is said by the pass, which paints that
                        // one again and logs why.
                        let _ = if live {
                            self.paint_live(fonts, key, painting, width, height)
                                .map(|_| ())
                        } else {
                            self.paint_kept(fonts, key, painting, width, height)
                                .map(|_| ())
                        };
                    }
                });
            }
        });
    }

    /// The faces, loaded once, with the project's own files added - each
    /// loaded once for the process.
    fn fonts_for<'a>(&self, project: &Model, held: &'a mut Option<Fonts>) -> &'a Fonts {
        let fonts = held.get_or_insert_with(Fonts::new);
        let mut loaded = self.loaded_files.lock().unwrap_or_else(|e| e.into_inner());
        for font in &project.fonts {
            if !font.path.is_empty() && loaded.insert(font.path.clone()) {
                fonts.add_file(Path::new(&font.path));
            }
        }
        fonts
    }

    fn remember(&self, key: u64, art: Art) {
        self.memo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, art);
    }
}

/// Everything the pixels depend on, hashed. Placement and timing are left
/// out on purpose; see the module docs.
///
/// FNV-1a over the bytes, as the other caches on disk are keyed, because a
/// file's name has to mean the same thing after a toolchain upgrade and the
/// standard hasher makes no such promise.
/// Each clip's key and painting, in the timeline's order - `None` for a
/// clip that is neither a title nor a shape - so a pass names each once.
/// A clip with no style of its own is painted in `plain`.
fn keys_of(project: &Model, width: u32, height: u32) -> Vec<Option<(u64, Painting<'_>)>> {
    placed(project)
        .into_iter()
        .map(|placed| {
            let painting = match &placed.item.content {
                Content::Title(title) => Painting::Title(title),
                Content::Shape(shape) => Painting::Shape(shape),
                _ => return None,
            };
            Some((key_of(project, painting, width, height), painting))
        })
        .collect()
}

/// An item of the sequence showing, where it is drawn.
struct Placed<'a> {
    id: ItemId,
    item: &'a Item,
    /// Its lane's place in the stack, from the bottom.
    index: usize,
    /// Whether its lane's pictures are drawn.
    visible: bool,
}

/// Every item of the sequence showing, lane by lane from the bottom and in
/// time order on each: the order the flattener lists them in.
fn placed(model: &Model) -> Vec<Placed<'_>> {
    let Some(sequence) = model.view.showing else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (index, (lane_id, _)) in model.lanes.index().of(sequence).enumerate() {
        let visible = model.lanes.get(lane_id).is_none_or(|lane| lane.visible);
        for (id, _, _) in model.items.index().on_lane(lane_id) {
            if let Some(item) = model.items.get(id) {
                out.push(Placed {
                    id,
                    item: item.as_ref(),
                    index,
                    visible,
                });
            }
        }
    }
    out
}

/// What a title or a shape shares with any picture of a file: its time,
/// its placement on a neutral base where it is keyed, its fades, its
/// moves and its blend, as the flattener works them out.
fn picture_base(item: &Item, index: usize) -> ExportClip {
    let p = &item.picture;
    let base = |n: &concat_doc::model::Animated<f64>, neutral: f64| {
        if n.is_keyed() { neutral } else { n.value }
    };
    let blank = ExportClip::blank(
        ClipKind::Image,
        item.start.display_seconds(),
        concat_doc::TickDur::from(item.len).display_seconds(),
        index,
    );
    ExportClip {
        scale: base(&p.scale, 1.0),
        offset_x: base(&p.offset_x, 0.0),
        offset_y: base(&p.offset_y, 0.0),
        rotation: base(&p.rotation, 0.0),
        stretch_x: base(&p.stretch_x, 1.0),
        stretch_y: base(&p.stretch_y, 1.0),
        opacity: base(&p.opacity, 1.0),
        fade_in: item.sound.fade_in.display_seconds(),
        fade_out: item.sound.fade_out.display_seconds(),
        motion: concat_export::flatten::motion(&p.motion),
        blend: concat_export::flatten::blend_name(&p.blend).to_owned(),
        ..blank
    }
}

fn key_of(project: &Model, painting: Painting<'_>, width: u32, height: u32) -> u64 {
    let mut bytes = Vec::new();
    // The style as written out field by field, in a stable order, with no
    // need to keep a serialiser in step with the struct. A shape's is prefixed, so
    // it can never collide with a title's.
    match painting {
        Painting::Title(style) => {
            bytes.extend_from_slice(format!("{style:?}").as_bytes());
        }
        Painting::Shape(style) => {
            bytes.extend_from_slice(b"shape:");
            bytes.extend_from_slice(format!("{style:?}").as_bytes());
        }
    }
    bytes.push(0);
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    for font in &project.fonts {
        bytes.extend_from_slice(font.family.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(font.path.as_bytes());
        bytes.push(0);
    }
    // Bumped when the painter's output changes for the same input, so stale
    // files are not mistaken for current ones. 3: left- and right-aligned
    // blocks moved to their anchors. 4: the sidecar gained each word's box,
    // for a per-word reveal effect to read back. 5: a title's lines are as
    // tall as their biggest word, and each look is painted on its own.
    // 6: a canvas grows past the frame to hold ink that runs past it, and
    // the sidecar says how big it is. 7: shapes gained their own width and
    // height, corners, gradients, outline placement, dashes, line ends,
    // shadows and trim. 8: the style is the document's own, written out.
    bytes.extend_from_slice(&8u32.to_le_bytes());
    crate::media::fnv1a(&bytes)
}

/// How many painted titles the directory keeps. Each is a frame-sized PNG -
/// larger for a title whose ink runs past the frame - mostly transparent, so this is tens of megabytes at most; above it the
/// oldest go, and a title that is still in use paints again.
const TITLES_KEPT: usize = 400;

/// A `width` by `height` straight-alpha canvas as the part of it with ink:
/// the smallest box holding every pixel that is not wholly transparent,
/// and where it sits. A caption is a line of words on a frame-sized
/// canvas, so this is a twentieth of the frame or less. None for pixels
/// that are not that canvas.
fn sparse_of(width: u32, height: u32, rgba: &[u8]) -> Option<SparseStill> {
    let row = width as usize * 4;
    if rgba.len() != row * height as usize {
        return None;
    }
    let (mut left, mut top, mut right, mut bottom) = (width, height, 0u32, 0u32);
    for (y, line) in rgba.chunks_exact(row.max(4)).enumerate() {
        let mut inked = line
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, px)| px[3] != 0);
        let Some((first, _)) = inked.next() else {
            continue;
        };
        let last = inked.next_back().map_or(first, |(x, _)| x);
        left = left.min(first as u32);
        right = right.max(last as u32 + 1);
        top = top.min(y as u32);
        bottom = y as u32 + 1;
    }
    if right <= left || bottom <= top {
        return Some(SparseStill {
            canvas: (width, height),
            at: (0, 0),
            block: Arc::new(Frame::transparent(1, 1)),
        });
    }
    let (bw, bh) = (right - left, bottom - top);
    let mut pixels = Vec::with_capacity(bw as usize * bh as usize * 4);
    for y in top..bottom {
        let from = y as usize * row + left as usize * 4;
        pixels.extend_from_slice(&rgba[from..from + bw as usize * 4]);
    }
    Some(SparseStill {
        canvas: (width, height),
        at: (left, top),
        block: Arc::new(Frame::from_rgba(bw, bh, pixels)?),
    })
}

/// Removes the oldest painted titles until `keep` remain. Sidecars go with
/// their PNGs. Best effort throughout: a file that will not go is left.
fn sweep(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut pngs: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    if pngs.len() <= keep {
        return;
    }
    pngs.sort();
    for (_, png) in pngs.iter().take(pngs.len() - keep) {
        let _ = std::fs::remove_file(png);
        let _ = std::fs::remove_file(png.with_extension("json"));
    }
}

fn read_block(side: &Path, width: u32, height: u32) -> Option<Art> {
    let text = std::fs::read_to_string(side).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let offset = |name: &str| value.get(name).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let rects: Vec<(i32, i32, u32, u32)> = value
        .get("words")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|word| {
            let word = word.as_array()?;
            Some((
                word.first()?.as_i64()? as i32,
                word.get(1)?.as_i64()? as i32,
                word.get(2)?.as_u64()? as u32,
                word.get(3)?.as_u64()? as u32,
            ))
        })
        .collect();
    // The canvas's size, which is the frame's unless it is said.
    let side_of = |name: &str, frame: u32| {
        value
            .get(name)
            .and_then(|v| v.as_u64())
            .map_or(frame, |v| v as u32)
    };
    let canvas = (side_of("cw", width), side_of("ch", height));
    Some(Art {
        block: (
            value.get("w")?.as_u64()? as u32,
            value.get("h")?.as_u64()? as u32,
        ),
        offset: (offset("x"), offset("y")),
        canvas,
        reveal: Arc::new(RevealMap::from_rects(canvas.0, canvas.1, &rects)),
    })
}

/// How many times a `width` by `height` frame a painting's canvas is,
/// along the axis it grew the most: one for a canvas that is the frame.
fn overscan_of(canvas: (u32, u32), width: u32, height: u32) -> f64 {
    (f64::from(canvas.0) / f64::from(width.max(1)))
        .max(f64::from(canvas.1) / f64::from(height.max(1)))
        .max(1.0)
}

/// A title's per-word reveal map, baked from the boxes its layout left
/// behind - a byproduct `concat-text` already computed, not new work.
fn reveal_of(words: &[WordRect], width: u32, height: u32) -> Arc<RevealMap> {
    let rects: Vec<(i32, i32, u32, u32)> = words
        .iter()
        .map(|w| (w.x, w.y, w.width, w.height))
        .collect();
    Arc::new(RevealMap::from_rects(width, height, &rects))
}

/// A colour as the painter reads it: `#rrggbb`, or `#rrggbbaa` when not
/// opaque.
fn hex(color: Color) -> String {
    concat_export::rgba::to_hex([color.r, color.g, color.b, color.a])
}

/// The document's title, field for field, in the painter's terms.
fn title_style(style: &Title) -> TitleStyle {
    // The words in their case, and the runs moved onto the characters
    // their own became in it.
    let (content, runs) = style.display();
    TitleStyle {
        content,
        font_family: style.font_family.clone(),
        font_size: style.font_size,
        font_weight: style.font_weight,
        italic: style.italic,
        underline: style.underline,
        strikethrough: style.strikethrough,
        color: hex(style.color),
        align: match style.align {
            TextAlign::Left => Align::Left,
            TextAlign::Center | TextAlign::Unknown(_) => Align::Center,
            TextAlign::Right => Align::Right,
        },
        stroke_width: style.stroke_width,
        stroke_color: hex(style.stroke_color),
        shadow: style.shadow,
        background: style.plate.map(hex).unwrap_or_default(),
        background_radius: style.plate_radius,
        background_padding_x: style.plate_padding_x,
        background_padding_y: style.plate_padding_y,
        line_height: style.line_height,
        max_width: style.max_width,
        max_height: style.max_height,
        tracking: style.tracking,
        runs: runs
            .into_iter()
            .map(|run| TitleRun {
                start: run.start as usize,
                end: run.end as usize,
                color: run.color.map(hex),
                font_weight: run.font_weight,
                italic: run.italic,
                underline: run.underline,
                strikethrough: run.strikethrough,
                font_scale: run.font_scale,
                stroke_color: run.stroke_color.map(hex),
            })
            .collect(),
    }
}

/// The document's shape, in the painter's terms: the box resolved, a
/// line's ends and the figure's own counts filled in.
fn shape_style(style: &ShapeStyle) -> concat_text::ShapeStyle {
    let head = |head: LineHead| match head {
        LineHead::None | LineHead::Unknown(_) => concat_text::Head::None,
        LineHead::Arrow => concat_text::Head::Arrow,
        LineHead::Triangle => concat_text::Head::Triangle,
        LineHead::Circle => concat_text::Head::Circle,
        LineHead::Bar => concat_text::Head::Bar,
    };
    let (width, height) = style.figure_box();
    concat_text::ShapeStyle {
        figure: match style.kind {
            ShapeKind::Square | ShapeKind::Unknown(_) => Figure::Square,
            ShapeKind::Circle => Figure::Circle,
            ShapeKind::Triangle => Figure::Triangle,
            ShapeKind::Parallelogram => Figure::Parallelogram,
            ShapeKind::Trapezoid => Figure::Trapezoid,
            ShapeKind::Line | ShapeKind::Arrow => Figure::Line,
            ShapeKind::Polygon => Figure::Polygon(style.sides),
            ShapeKind::Star => Figure::Star(style.points, style.inner_radius as f32),
            ShapeKind::Heart => Figure::Heart,
        },
        width,
        height,
        corner_radius: style.corner_radius,
        fill: style.fill.map(hex).unwrap_or_default(),
        gradient: style
            .gradient
            .as_ref()
            .map(|gradient| concat_text::Gradient {
                radial: gradient.radial,
                color: hex(gradient.color),
                angle: gradient.angle,
            }),
        stroke: style.stroke.map(hex).unwrap_or_default(),
        stroke_width: style.stroke_width,
        align: match style.stroke_align {
            StrokeAlign::Center | StrokeAlign::Unknown(_) => concat_text::StrokeAlign::Center,
            StrokeAlign::Inside => concat_text::StrokeAlign::Inside,
            StrokeAlign::Outside => concat_text::StrokeAlign::Outside,
        },
        dash: match style.dash {
            StrokeDash::Solid | StrokeDash::Unknown(_) => concat_text::Dash::Solid,
            StrokeDash::Dashed => concat_text::Dash::Dashed,
            StrokeDash::Dotted => concat_text::Dash::Dotted,
        },
        start_head: head(style.start()),
        end_head: head(style.end()),
        shadow: style.shadow.as_ref().map(|shadow| concat_text::Shadow {
            color: hex(shadow.color),
            blur: shadow.blur,
            distance: shadow.distance,
            angle: shadow.angle,
        }),
        trim_start: style.trim_start,
        trim_end: style.trim_end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_doc::{Doc, at, long};
    use concat_doc::commands::{Command, KeyProperty, RunStyle};
    use concat_doc::model::{Ease, TextCase};

    fn scratch(name: &str) -> AppDirs {
        let dir =
            std::env::temp_dir().join(format!("concat-titles-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        AppDirs {
            config: dir.join("config"),
            data: dir.join("data"),
        }
    }

    fn title(doc: &mut Doc, start: f64, seconds: f64, offset_y: Option<f64>) -> ItemId {
        doc.item(Command::AddTextClip {
            start: at(start),
            len: Some(long(seconds)),
            lane: None,
            above: true,
            offset_y,
            title: None,
        })
    }

    fn transform(doc: &mut Doc, item: ItemId, scale: Option<f64>, stretch: Option<(f64, f64)>) {
        doc.apply(Command::SetClipTransform {
            item,
            offset_x: None,
            offset_y: None,
            rotation: None,
            scale,
            stretch_x: stretch.map(|s| s.0),
            stretch_y: stretch.map(|s| s.1),
        });
    }

    /// A title clip carrying a stretch comes back unstretched: the words
    /// wrap, and a squashed glyph was never wanted.
    /// https://github.com/jub0t/Concat/issues/119
    #[test]
    fn a_title_is_never_stretched_whatever_its_clip_says() {
        let dirs = scratch("title");
        let mut doc = Doc::new();
        let id = title(&mut doc, 0.0, 3.0, None);
        transform(&mut doc, id, Some(1.03), Some((0.46, 2.2)));
        let out = Titles::new(&dirs).clips(doc.model(), 640, 360);
        assert_eq!((out[0].clip.stretch_x, out[0].clip.stretch_y), (1.0, 1.0));
        assert_eq!(out[0].clip.scale, 1.03, "the scale is honoured");

        // Nor by keys: a keyed width on a title is dropped with its
        // constant, and its other keys still play.
        for (property, value) in [(KeyProperty::StretchX, 3.0), (KeyProperty::Scale, 2.0)] {
            doc.apply(Command::SetClipKey {
                item: id,
                property,
                at: at(1.5),
                value,
                ease: Ease::LINEAR,
            });
        }
        let out = Titles::new(&dirs).clips(doc.model(), 640, 360);
        let names: Vec<&str> = out[0]
            .clip
            .animation
            .iter()
            .map(|key| key.property.as_str())
            .collect();
        assert_eq!(names, ["scale"]);
        let _ = std::fs::remove_dir_all(dirs.data.parent().expect("a scratch root"));
    }

    /// A keyed position plays from a neutral base, as a media clip's does,
    /// so the title is drawn where its keys put it (#512).
    #[test]
    fn a_keyed_title_plays_its_keys_from_a_neutral_base() {
        let dirs = scratch("keyed");
        let mut doc = Doc::new();
        let id = title(&mut doc, 0.0, 2.0, Some(0.3));
        for (property, when, value) in [
            (KeyProperty::OffsetY, 0.0, -0.1),
            (KeyProperty::OffsetY, 2.0, -0.1),
            (KeyProperty::Opacity, 0.0, 0.5),
        ] {
            doc.apply(Command::SetClipKey {
                item: id,
                property,
                at: at(when),
                value,
                ease: Ease::LINEAR,
            });
        }
        let out = Titles::new(&dirs).clips(doc.model(), 640, 360);
        let clip = &out[0].clip;
        assert_eq!(clip.offset_y, 0.0, "keys, not 0.3 plus keys");
        assert_eq!(clip.opacity, 1.0);
        assert_eq!(clip.scale, 1.0);
        assert!(!clip.animation.is_empty(), "the keys travel with it");
        let _ = std::fs::remove_dir_all(dirs.data.parent().expect("a scratch root"));
    }

    /// Some words styled are a different picture, with the runs on the
    /// characters the painter is given: in capitals "ß" is two of them.
    #[test]
    fn styled_words_are_a_new_picture_and_follow_the_case() {
        let dirs = scratch("styled");
        let mut doc = Doc::new();
        let id = doc.item(Command::AddTextClip {
            start: at(0.0),
            len: Some(long(3.0)),
            lane: None,
            above: false,
            offset_y: None,
            title: Some(Title {
                content: "straße rot".into(),
                ..Title::default()
            }),
        });
        let titles = Titles::new(&dirs);
        let plain = titles.clips(doc.model(), 640, 360)[0].clip.path.clone();
        doc.apply(Command::StyleTextRange {
            item: id,
            start: 7,
            end: 10,
            style: RunStyle {
                color: Some(Color::rgb(255, 0, 0)),
                ..RunStyle::default()
            },
        });
        let styled = titles.clips(doc.model(), 640, 360)[0].clip.path.clone();
        assert_ne!(styled, plain, "a different picture");
        let Content::Title(text) = &doc.model().items.get(id).expect("kept").content else {
            panic!("a title");
        };
        let mut text = text.clone();
        text.case = TextCase::Upper;
        let painted = title_style(&text);
        assert_eq!(painted.content, "STRASSE ROT");
        assert_eq!((painted.runs[0].start, painted.runs[0].end), (8, 11));
        assert_eq!(painted.runs[0].color.as_deref(), Some("#ff0000"));
        let _ = std::fs::remove_dir_all(dirs.data.parent().expect("a scratch root"));
    }

    /// The window's look paints nothing; once a painter has been through,
    /// every title is there.
    #[test]
    fn ready_paints_nothing_and_says_what_is_missing() {
        let dirs = scratch("ready");
        let mut doc = Doc::new();
        for start in [0.0, 2.0, 4.0] {
            title(&mut doc, start, 1.5, None);
        }
        let titles = Titles::new(&dirs);
        let (none, complete) = titles.ready(doc.model(), 640, 360);
        assert!(none.is_empty() && !complete);
        assert_eq!(titles.clips(doc.model(), 640, 360).len(), 3);
        let (all, complete) = titles.ready(doc.model(), 640, 360);
        assert_eq!(all.len(), 3);
        assert!(complete);
        let _ = std::fs::remove_dir_all(dirs.data.parent().expect("a scratch root"));
    }

    /// An export stopped while its titles paint gets nothing back.
    #[test]
    fn a_stopped_export_paints_no_titles() {
        let dirs = scratch("stopped");
        let mut doc = Doc::new();
        for start in [0.0, 2.0, 4.0] {
            title(&mut doc, start, 1.5, None);
        }
        let titles = Titles::new(&dirs);
        let stop = AtomicBool::new(true);
        assert!(titles.clips_unless(doc.model(), 1280, 720, &stop).is_none());
        assert!(titles.ready(doc.model(), 1280, 720).0.is_empty());
        stop.store(false, Ordering::Relaxed);
        let all = titles
            .clips_unless(doc.model(), 1280, 720, &stop)
            .expect("not stopped");
        assert_eq!(all.len(), 3);
        assert!(all.iter().all(|t| t.clip.media_width == Some(1280)));
        let _ = std::fs::remove_dir_all(dirs.data.parent().expect("a scratch root"));
    }

    #[test]
    fn a_title_rejoins_as_a_still() {
        let dirs = scratch("rejoins");
        let mut doc = Doc::new();
        let id = title(&mut doc, 2.0, 3.0, Some(0.3));
        let titles = Titles::new(&dirs);
        let out = titles.clips(doc.model(), 640, 360);
        assert_eq!(out.len(), 1);
        let title = &out[0];
        assert_eq!(title.clip_id, id);
        assert!(matches!(title.clip.kind, ClipKind::Image));
        assert!((title.clip.start - 2.0).abs() < 1e-9 && (title.clip.duration - 3.0).abs() < 1e-9);
        assert_eq!(title.clip.offset_y, 0.3);
        assert_eq!(title.clip.media_width, Some(640));
        assert!(title.block.0 > 0 && title.block.1 > 0);
        assert_eq!(title.offset, (0, 0));
        let reveal = title.clip.reveal_map.as_ref().expect("a title reveals");
        assert!(reveal.gray.contains(&0) && reveal.gray.contains(&255));
        let painted = Path::new(&title.clip.path);
        assert!(painted.is_file(), "the PNG is on disk");
        let again = titles.clips(doc.model(), 640, 360);
        assert_eq!(again[0].clip.path, title.clip.path, "painted once");
        let wide = titles.clips(doc.model(), 1280, 720);
        assert_ne!(wide[0].clip.path, title.clip.path);
        let _ = std::fs::remove_dir_all(dirs.data.parent().expect("a scratch root"));
    }

    #[test]
    fn live_paintings_go_least_recently_used_first() {
        let art = || Art {
            block: (1, 1),
            offset: (0, 0),
            canvas: (4, 4),
            reveal: Arc::new(RevealMap::from_rects(1, 1, &[])),
        };
        let frame = || {
            Arc::new(SparseStill {
                canvas: (4, 4),
                at: (0, 0),
                block: Arc::new(Frame::transparent(4, 4)),
            })
        };
        let size = frame().bytes();
        let mut live = LivePaintings::new(2 * size, 3 * size);
        live.begin();
        live.keep(1, art(), frame());
        live.keep(2, art(), frame());
        live.begin();
        assert!(live.get(1).is_some());
        live.begin();
        live.keep(3, art(), frame());
        assert!(live.get(1).is_some(), "the recently used stays");
        assert!(live.get(2).is_none(), "the least recently used goes");
    }

    #[test]
    fn a_shape_rejoins_as_a_still_and_keeps_its_stretch() {
        let dirs = scratch("shape");
        let mut doc = Doc::new();
        let id = doc.item(Command::AddShapeClip {
            start: at(0.0),
            len: Some(long(3.0)),
            name: "Square".to_owned(),
            style: None,
            lane: None,
            above: true,
        });
        transform(&mut doc, id, None, Some((2.0, 0.5)));
        let out = Titles::new(&dirs).clips(doc.model(), 640, 360);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].clip_id, id);
        assert_eq!((out[0].clip.stretch_x, out[0].clip.stretch_y), (2.0, 0.5));
        assert_eq!(out[0].block, (126, 126), "35 % of 360, squared");
        assert!(out[0].clip.reveal_map.is_none());
        let _ = std::fs::remove_dir_all(dirs.data.parent().expect("a scratch root"));
    }
}

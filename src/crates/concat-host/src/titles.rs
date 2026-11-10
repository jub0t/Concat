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
use std::sync::{Arc, Mutex};

use concat_core::frame::Frame;
use concat_core::shader::RevealMap;

use concat_export::flatten::export_base;
use concat_export::{ClipKind, ExportClip};
use concat_project::model::{
    ClipKind as ModelClipKind, KeyProperty, Project, ShapeKind, ShapeStyle, TextAlign, TextStyle,
};
use concat_text::{Align, Figure, Fonts, TitleRun, TitleStyle, WordRect};

use crate::dirs::AppDirs;

/// One title or shape, ready for the compositor.
#[derive(Clone)]
pub struct TitleClip {
    /// The text or shape clip this was painted from.
    pub clip_id: String,
    /// The image clip that stands in for it.
    pub clip: ExportClip,
    /// The painted block's size in frame pixels, for an outline on a monitor.
    pub block: (u32, u32),
    /// Where that block's centre is, as an offset in frame pixels from the
    /// clip's own centre: zero for a centred title, half a block to the
    /// right for a left-aligned one, whose block starts at the clip's
    /// position. See `concat_text::Align`.
    pub offset: (i32, i32),
    /// The pixels, when the title was painted live rather than to disk:
    /// what the monitor is to show under `clip.path`, a name no file has.
    /// See [`Titles::clips_live`].
    pub frame: Option<Arc<Frame>>,
}

/// What is painted: a title's words or a shape's figure. One painter for
/// both, since they are cached, keyed and handed on the same way.
#[derive(Clone, Copy)]
enum Painting<'a> {
    Title(&'a TextStyle),
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
}

/// The live paintings kept, in bytes of pixels, beyond the ones the
/// timeline is showing; see [`LivePaintings`]. Each is a picture the
/// frame's size - 8 MB at 1080p, 33 MB at 4K - so a count cost four times
/// as much on a 4K timeline as on an HD one. 128 MB keeps sixteen HD
/// paintings or four 4K ones.
const LIVE_BYTES: usize = 128 * 1024 * 1024;

/// The most the paintings of the titles on the timeline may hold between
/// them, in bytes of pixels: 64 HD titles, or 256 at the monitor's half
/// size. Past it the first painted are kept and the rest painted again
/// each time; see [`LivePaintings::keep`].
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
    frame: Arc<Frame>,
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
    fn get(&mut self, key: u64) -> Option<(Art, Arc<Frame>)> {
        let pass = self.pass;
        self.kept.get_mut(&key).map(|painting| {
            painting.used = pass;
            (painting.art.clone(), Arc::clone(&painting.frame))
        })
    }

    /// Keeps a painting under `key`. Over the budget, the least recently
    /// used go first, never one this pass has asked for. Over the ceiling
    /// with nothing else left, the one just painted is not kept: the ones
    /// painted before it stay for the next pass, rather than each pushing
    /// out the one it will want next. The newest is always kept alone.
    fn keep(&mut self, key: u64, art: Art, frame: Arc<Frame>) {
        let size = frame.pixels().len();
        let painting = LivePainting {
            art,
            frame,
            used: self.pass,
        };
        if let Some(old) = self.kept.insert(key, painting) {
            self.bytes -= old.frame.pixels().len();
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
            self.bytes -= gone.frame.pixels().len();
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
        }
    }

    /// Every text and shape clip on the active timeline, as an image clip
    /// each, for a `width` × `height` frame. One that fails to paint is left
    /// out and said once on stderr; the rest of the edit still renders.
    pub fn clips(&self, project: &Project, width: u32, height: u32) -> Vec<TitleClip> {
        self.clips_with(project, width, height, false)
    }

    /// [`Titles::clips`] for a monitor showing a change as it is made:
    /// every title is painted in memory at `width` by `height` - the
    /// monitor's own size, not the output's - and comes back with its
    /// pixels under a name no file has, for the monitor to hold. Nothing
    /// touches the disk, which is what makes a drag smooth: a PNG per
    /// pointer step, encoded, written and decoded again, was the lag.
    pub fn clips_live(&self, project: &Project, width: u32, height: u32) -> Vec<TitleClip> {
        self.clips_with(project, width, height, true)
    }

    fn clips_with(&self, project: &Project, width: u32, height: u32, live: bool) -> Vec<TitleClip> {
        if live {
            self.live.lock().unwrap_or_else(|e| e.into_inner()).begin();
        }
        let (plain_text, plain_shape) = (TextStyle::default(), ShapeStyle::default());
        let keys = self.paint_ahead(project, width, height, live, (&plain_text, &plain_shape));
        let timeline = project.active();
        let mut out = Vec::new();
        for (clip, key) in timeline.clips.iter().zip(keys) {
            let Some((key, painting)) = key else {
                continue;
            };
            let Some(index) = timeline
                .tracks
                .iter()
                .position(|track| track.id == clip.track_id)
            else {
                continue;
            };
            let track = &timeline.tracks[index];
            let painted = if live {
                self.painted_live(project, key, painting, width, height)
                    .map(|(path, art, frame)| (path, art, Some(frame)))
            } else {
                self.painted(project, key, painting, width, height)
                    .map(|(path, art)| (path, art, None))
            };
            let (path, art, frame) = match painted {
                Ok(art) => art,
                Err(error) => {
                    log::warn!("title {}: {error}", clip.id);
                    continue;
                }
            };
            out.push(TitleClip {
                clip_id: clip.id.clone(),
                clip: ExportClip {
                    path: path.to_string_lossy().into_owned(),
                    hidden: !track.visible,
                    muted: true,
                    volume: 0.0,
                    animation: concat_export::flatten::export_keys(clip),
                    // What a media clip carries in the flattening and a
                    // title or a shape would otherwise lose here, starting
                    // from blank: the Animate moves, the mask and the
                    // fades. Without them no In, Out or Loop played on any
                    // title, caption or shape.
                    motion: clip.motion.clone(),
                    masks: clip.masks.clone(),
                    fade_in: clip.fade_in,
                    fade_out: clip.fade_out,
                    flip_h: clip.flip_h,
                    flip_v: clip.flip_v,
                    blend: clip.blend.clone(),
                    effects: clip.video_effects.clone(),
                    // A keyed property's keys travel absolute, on a
                    // neutral base, as a media clip's do: the clip's own
                    // offset under keys put the title at the two added,
                    // away from the gizmo and often off the frame (#512).
                    scale: export_base(clip, KeyProperty::Scale),
                    offset_x: export_base(clip, KeyProperty::OffsetX),
                    offset_y: export_base(clip, KeyProperty::OffsetY),
                    rotation: export_base(clip, KeyProperty::Rotation),
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
                        Painting::Shape(_) => clip.stretch_x,
                    },
                    stretch_y: match painting {
                        Painting::Title(_) => 1.0,
                        Painting::Shape(_) => clip.stretch_y,
                    },
                    // The style's own opacity multiplies the clip's: a
                    // half-transparent title fades to half, not to solid.
                    opacity: match painting {
                        Painting::Title(text) => {
                            (export_base(clip, KeyProperty::Opacity) * text.opacity).clamp(0.0, 1.0)
                        }
                        Painting::Shape(_) => {
                            export_base(clip, KeyProperty::Opacity).clamp(0.0, 1.0)
                        }
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
                    ..ExportClip::blank(ClipKind::Image, clip.start, clip.duration, index)
                },
                block: art.block,
                offset: art.offset,
                frame,
            });
        }
        out
    }

    /// One style at one size, painted in memory: the name the monitor is
    /// to hold the pixels under, the block, and the pixels.
    fn painted_live(
        &self,
        project: &Project,
        key: u64,
        painting: Painting<'_>,
        width: u32,
        height: u32,
    ) -> Result<(PathBuf, Art, Arc<Frame>), String> {
        if let Some(found) = self.live_painting(key) {
            return Ok(found);
        }
        let mut fonts = self.fonts.lock().unwrap_or_else(|e| e.into_inner());
        let fonts = self.fonts_for(project, &mut fonts);
        self.paint_live(fonts, key, painting, width, height)
    }

    /// The live painting kept under `key`, if there is one.
    fn live_painting(&self, key: u64) -> Option<(PathBuf, Art, Arc<Frame>)> {
        let path = PathBuf::from(format!("memory://titles/{key:016x}"));
        let (art, frame) = self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)?;
        Some((path, art, frame))
    }

    /// Paints one style in memory with `fonts`, and keeps it under `key`.
    fn paint_live(
        &self,
        fonts: &Fonts,
        key: u64,
        painting: Painting<'_>,
        width: u32,
        height: u32,
    ) -> Result<(PathBuf, Art, Arc<Frame>), String> {
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
        let frame = Arc::new(
            Frame::from_rgba(rendered.width, rendered.height, rendered.rgba)
                .ok_or_else(|| "the painter returned a frame of the wrong size".to_owned())?,
        );
        self.live.lock().unwrap_or_else(|e| e.into_inner()).keep(
            key,
            art.clone(),
            Arc::clone(&frame),
        );
        Ok((path, art, frame))
    }

    /// The PNG for one style at one frame size, painted if it is not on disk
    /// yet, and the block it holds.
    fn painted(
        &self,
        project: &Project,
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
    /// the project's own, and only read while the titles paint.
    ///
    /// Gives back each clip's key and painting, in the timeline's order -
    /// `None` for a clip that is neither - so the pass names each once.
    /// A clip with no style of its own is painted in `plain`.
    fn paint_ahead<'a>(
        &self,
        project: &'a Project,
        width: u32,
        height: u32,
        live: bool,
        plain: (&'a TextStyle, &'a ShapeStyle),
    ) -> Vec<Option<(u64, Painting<'a>)>> {
        let keys: Vec<Option<(u64, Painting<'a>)>> = project
            .active()
            .clips
            .iter()
            .map(|clip| {
                let painting = match clip.kind {
                    ModelClipKind::Text => Painting::Title(clip.text.as_ref().unwrap_or(plain.0)),
                    ModelClipKind::Shape => Painting::Shape(clip.shape.as_ref().unwrap_or(plain.1)),
                    _ => return None,
                };
                Some((key_of(project, painting, width, height), painting))
            })
            .collect();
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
            return keys;
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
        keys
    }

    /// The faces, loaded once, with the project's own files added - each
    /// loaded once for the process.
    fn fonts_for<'a>(&self, project: &Project, held: &'a mut Option<Fonts>) -> &'a Fonts {
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
fn key_of(project: &Project, painting: Painting<'_>, width: u32, height: u32) -> u64 {
    let mut bytes = Vec::new();
    // The style as JSON: every field, in a stable order, with no need to
    // keep a serialiser in step with the struct. A shape's is prefixed, so
    // it can never collide with a title's.
    match painting {
        Painting::Title(style) => {
            bytes.extend_from_slice(serde_json::to_string(style).unwrap_or_default().as_bytes());
        }
        Painting::Shape(style) => {
            bytes.extend_from_slice(b"shape:");
            bytes.extend_from_slice(serde_json::to_string(style).unwrap_or_default().as_bytes());
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
    // shadows and trim.
    bytes.extend_from_slice(&7u32.to_le_bytes());
    crate::media::fnv1a(&bytes)
}

/// How many painted titles the directory keeps. Each is a frame-sized PNG -
/// larger for a title whose ink runs past the frame - mostly transparent, so this is tens of megabytes at most; above it the
/// oldest go, and a title that is still in use paints again.
const TITLES_KEPT: usize = 400;

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

/// The document's style, field for field, in the painter's terms.
fn title_style(style: &TextStyle) -> TitleStyle {
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
        color: style.color.clone(),
        align: match style.align {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        },
        stroke_width: style.stroke_width,
        stroke_color: style.stroke_color.clone(),
        shadow: style.shadow,
        background: style.background.clone(),
        background_radius: style.background_radius,
        background_padding_x: style.background_padding_x,
        background_padding_y: style.background_padding_y,
        line_height: style.line_height,
        max_width: style.max_width,
        max_height: style.max_height,
        tracking: style.tracking,
        runs: runs
            .into_iter()
            .map(|run| TitleRun {
                start: run.start,
                end: run.end,
                color: run.style.color,
                font_weight: run.style.font_weight,
                italic: run.style.italic,
                underline: run.style.underline,
                strikethrough: run.style.strikethrough,
                font_scale: run.style.font_scale,
                stroke_color: run.style.stroke_color,
            })
            .collect(),
    }
}

/// The document's shape, in the painter's terms: the box resolved, a
/// line's ends and the figure's own counts filled in.
fn shape_style(style: &ShapeStyle) -> concat_text::ShapeStyle {
    use concat_project::model::{LineHead, StrokeAlign, StrokeDash};
    let head = |head: LineHead| match head {
        LineHead::None => concat_text::Head::None,
        LineHead::Arrow => concat_text::Head::Arrow,
        LineHead::Triangle => concat_text::Head::Triangle,
        LineHead::Circle => concat_text::Head::Circle,
        LineHead::Bar => concat_text::Head::Bar,
    };
    let (width, height) = style.figure_box();
    concat_text::ShapeStyle {
        figure: match style.kind {
            ShapeKind::Square => Figure::Square,
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
        fill: style.fill.clone(),
        gradient: style
            .gradient
            .as_ref()
            .map(|gradient| concat_text::Gradient {
                radial: gradient.radial,
                color: gradient.color.clone(),
                angle: gradient.angle,
            }),
        stroke: style.stroke.clone(),
        stroke_width: style.stroke_width,
        align: match style.stroke_align {
            StrokeAlign::Center => concat_text::StrokeAlign::Center,
            StrokeAlign::Inside => concat_text::StrokeAlign::Inside,
            StrokeAlign::Outside => concat_text::StrokeAlign::Outside,
        },
        dash: match style.dash {
            StrokeDash::Solid => concat_text::Dash::Solid,
            StrokeDash::Dashed => concat_text::Dash::Dashed,
            StrokeDash::Dotted => concat_text::Dash::Dotted,
        },
        start_head: head(style.start()),
        end_head: head(style.end()),
        shadow: style.shadow.as_ref().map(|shadow| concat_text::Shadow {
            color: shadow.color.clone(),
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
    use concat_project::{Command, Editor};

    fn scratch(name: &str) -> AppDirs {
        let dir =
            std::env::temp_dir().join(format!("concat-titles-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        AppDirs {
            config: dir.join("config"),
            data: dir.join("data"),
        }
    }

    /// A title clip carrying a stretch - what 0.2.2's side grips wrote to
    /// make a long caption fit - comes back unstretched: the words wrap
    /// now, and a squashed glyph was never wanted.
    /// https://github.com/jub0t/Concat/issues/119
    #[test]
    fn a_title_is_never_stretched_whatever_its_clip_says() {
        let dirs = scratch("title");
        let mut editor = Editor::new();
        let id = editor
            .apply(Command::AddTextClip {
                above: false,
                track_id: None,
                start: 0.0,
                style: None,
                duration: Some(3.0),
                offset_y: None,
            })
            .expect("a title is added")
            .created_id
            .expect("with an id");
        editor
            .apply(Command::SetClipTransform {
                clip_id: id.clone(),
                scale: Some(1.03),
                offset_x: None,
                offset_y: None,
                rotation: None,
                stretch_x: Some(0.46),
                stretch_y: Some(2.2),
            })
            .expect("the stretch a 0.2.2 grip left behind");
        let clip = editor.project().active().clip(&id).expect("kept");
        assert_eq!(
            (clip.stretch_x, clip.stretch_y),
            (0.46, 2.2),
            "it is on the clip"
        );

        let out = Titles::new(&dirs).clips(editor.project(), 640, 360);
        assert_eq!((out[0].clip.stretch_x, out[0].clip.stretch_y), (1.0, 1.0));
        assert_eq!(
            out[0].clip.scale, 1.03,
            "the scale is honoured; only the stretch is not"
        );
    }

    /// A text clip comes back as an image clip on its own track, pointing
    /// at a PNG on disk that is the frame's size, and a second ask for the
    /// same title paints nothing new.
    /// A keyed position plays from a neutral base, as a media clip's does,
    /// so the title is drawn where its keys - and the gizmo - put it, not
    /// at the clip's own offset added to them (#512).
    #[test]
    fn a_keyed_title_plays_its_keys_from_a_neutral_base() {
        let dirs = scratch("a_keyed_titl");
        let mut editor = Editor::new();
        let id = editor
            .apply(Command::AddTextClip {
                above: false,
                track_id: None,
                start: 0.0,
                style: None,
                duration: Some(2.0),
                offset_y: Some(0.3),
            })
            .expect("a title is added")
            .created_id
            .expect("with an id");
        for (property, at, value) in [
            (KeyProperty::OffsetY, 0.0, -0.1),
            (KeyProperty::OffsetY, 1.0, -0.1),
            (KeyProperty::Opacity, 0.0, 0.5),
        ] {
            editor
                .apply(Command::SetClipKey {
                    clip_id: id.clone(),
                    property,
                    at,
                    value,
                    ease: Default::default(),
                })
                .expect("a key is set");
        }

        let out = Titles::new(&dirs).clips(editor.project(), 640, 360);
        let title = &out[0].clip;
        assert_eq!(title.offset_y, 0.0, "keys, not 0.3 plus keys");
        assert_eq!(
            title.opacity, 1.0,
            "the style's own opacity, times the keys"
        );
        assert_eq!(title.offset_x, 0.0);
        assert_eq!(title.scale, 1.0, "an unkeyed property keeps the clip's own");
        assert!(!title.animation.is_empty(), "the keys travel with it");
        let _ = std::fs::remove_dir_all(dirs.data.parent().expect("a scratch root"));
    }

    /// Some words styled are a different picture, painted again, with the
    /// runs on the characters the painter is given - in capitals, "ß" is
    /// two of them, and the run after it moves along by one.
    #[test]
    fn styled_words_are_a_new_picture_and_follow_the_case() {
        let dirs = scratch("styled_words");
        let mut editor = Editor::new();
        let id = editor
            .apply(Command::AddTextClip {
                above: false,
                track_id: None,
                start: 0.0,
                style: Some(TextStyle {
                    content: "straße rot".into(),
                    ..TextStyle::default()
                }),
                duration: Some(3.0),
                offset_y: None,
            })
            .expect("a title is added")
            .created_id
            .expect("with an id");
        let titles = Titles::new(&dirs);
        let plain = titles.clips(editor.project(), 640, 360)[0]
            .clip
            .path
            .clone();
        editor
            .apply(Command::StyleTextRange {
                clip_id: id.clone(),
                start: 7,
                end: 10,
                patch: concat_project::model::RunStyle {
                    color: Some("#ff0000".into()),
                    ..Default::default()
                },
            })
            .expect("styles");
        let styled = titles.clips(editor.project(), 640, 360)[0]
            .clip
            .path
            .clone();
        assert_ne!(styled, plain, "a different picture");

        let mut text = editor
            .project()
            .active()
            .clip(&id)
            .unwrap()
            .text
            .clone()
            .unwrap();
        text.text_case = concat_project::model::TextCase::Upper;
        let painted = title_style(&text);
        assert_eq!(painted.content, "STRASSE ROT");
        assert_eq!((painted.runs[0].start, painted.runs[0].end), (8, 11));
        assert_eq!(painted.runs[0].color.as_deref(), Some("#ff0000"));
        let _ = std::fs::remove_dir_all(dirs.data.parent().expect("a scratch root"));
    }

    #[test]
    fn a_title_rejoins_as_a_still() {
        let dirs = scratch("a_title_rejo");
        let mut editor = Editor::new();
        let id = editor
            .apply(Command::AddTextClip {
                above: false,
                track_id: None,
                start: 2.0,
                style: None,
                duration: Some(3.0),
                offset_y: Some(0.3),
            })
            .expect("a title is added")
            .created_id
            .expect("with an id");

        let titles = Titles::new(&dirs);
        let out = titles.clips(editor.project(), 640, 360);
        assert_eq!(out.len(), 1);
        let title = &out[0];
        assert_eq!(title.clip_id, id);
        assert!(matches!(title.clip.kind, ClipKind::Image));
        assert_eq!((title.clip.start, title.clip.duration), (2.0, 3.0));
        assert_eq!(title.clip.offset_y, 0.3);
        assert_eq!(title.clip.media_width, Some(640));
        assert!(title.block.0 > 0 && title.block.1 > 0);
        assert_eq!(
            title.offset,
            (0, 0),
            "a centred title's block is on the clip"
        );
        // "Your text": two words, so their reveal order spans 0 to 255.
        let reveal = title.clip.reveal_map.as_ref().expect("a title reveals");
        assert_eq!((reveal.width, reveal.height), (640, 360));
        assert!(reveal.gray.contains(&0));
        assert!(reveal.gray.contains(&255), "a second word reaches the top");
        let painted = Path::new(&title.clip.path);
        assert!(painted.is_file(), "the PNG is on disk");
        let stamp = std::fs::metadata(painted).and_then(|m| m.modified()).ok();

        // Same words, same frame: same file, untouched.
        let again = titles.clips(editor.project(), 640, 360);
        assert_eq!(again[0].clip.path, title.clip.path);
        assert_eq!(
            std::fs::metadata(painted).and_then(|m| m.modified()).ok(),
            stamp
        );

        // A different frame is a different picture.
        let wide = titles.clips(editor.project(), 1280, 720);
        assert_ne!(wide[0].clip.path, title.clip.path);

        // Aligned left, the block starts at the clip's position and its
        // centre is reported half a block to the right - and a second
        // painter, reading the sidecar cold, says the same.
        let style = concat_project::model::TextStyle {
            align: concat_project::model::TextAlign::Left,
            ..Default::default()
        };
        editor
            .apply(Command::UpdateClip {
                clip_id: id.clone(),
                patch: concat_project::commands::ClipPatch {
                    text: Some(Some(style)),
                    ..Default::default()
                },
            })
            .expect("aligns");
        let left = titles.clips(editor.project(), 640, 360);
        assert!(left[0].offset.0 > 0);
        assert!((left[0].offset.0 as u32).abs_diff(left[0].block.0 / 2) <= 1);
        let cold = Titles::new(&dirs).clips(editor.project(), 640, 360);
        assert_eq!(cold[0].offset, left[0].offset);
        assert_eq!(cold[0].block, left[0].block);
        // The sidecar carries the words too, so a cold read reveals the
        // same order as the one that painted them.
        assert_eq!(
            cold[0].clip.reveal_map.as_ref().map(|r| &*r.gray),
            left[0].clip.reveal_map.as_ref().map(|r| &*r.gray)
        );
        let _ = std::fs::remove_dir_all(dirs.data.parent().unwrap());
    }

    /// A title whose box is pulled wider than the frame rejoins on a canvas
    /// grown to hold it, said to be that many frames across so it is drawn
    /// one to one; its reveal map is the canvas's size, and a painter
    /// reading the sidecar cold, and the live painter, say the same.
    #[test]
    fn a_title_wider_than_the_frame_rejoins_on_its_grown_canvas() {
        let dirs = scratch("a_title_wide");
        let mut editor = Editor::new();
        let style = concat_project::model::TextStyle {
            content: "a box wider than the frame".to_owned(),
            background: "#2040a0".to_owned(),
            max_width: 1.5,
            ..Default::default()
        };
        editor
            .apply(Command::AddTextClip {
                above: false,
                track_id: None,
                start: 0.0,
                style: Some(style),
                duration: Some(3.0),
                offset_y: None,
            })
            .expect("a title is added");

        let titles = Titles::new(&dirs);
        let out = titles.clips(editor.project(), 640, 360);
        let clip = &out[0].clip;
        let (canvas_w, canvas_h) = (
            clip.media_width.expect("a width"),
            clip.media_height.expect("a height"),
        );
        assert!(canvas_w >= 960, "the box and its shadow: {canvas_w}");
        assert_eq!((canvas_w - 640) % 2, 0, "the same margin either side");
        let overscan = (f64::from(canvas_w) / 640.0).max(f64::from(canvas_h) / 360.0);
        assert_eq!(clip.overscan, overscan);
        let reveal = clip.reveal_map.as_ref().expect("a title reveals");
        assert_eq!((reveal.width, reveal.height), (canvas_w, canvas_h));
        assert_eq!(out[0].block.0, 960, "the block is the box, in frame pixels");

        let cold = Titles::new(&dirs).clips(editor.project(), 640, 360);
        assert_eq!(cold[0].clip.media_width, clip.media_width);
        assert_eq!(cold[0].clip.overscan, clip.overscan);
        assert_eq!(
            cold[0]
                .clip
                .reveal_map
                .as_ref()
                .map(|r| (r.width, r.height)),
            Some((canvas_w, canvas_h))
        );
        let live = titles.clips_live(editor.project(), 640, 360);
        let frame = live[0].frame.as_ref().expect("painted live");
        assert_eq!((frame.width(), frame.height()), (canvas_w, canvas_h));
        assert_eq!(live[0].clip.overscan, clip.overscan);
        let _ = std::fs::remove_dir_all(dirs.data.parent().unwrap());
    }

    /// More titles than the byte budget holds are all kept while the
    /// timeline shows them: a second live pass paints none of them again.
    /// Kept first in, first out, each was thrown out just before it was
    /// wanted, and every pass painted them all (#261).
    #[test]
    fn a_timeline_of_titles_past_the_budget_is_painted_once() {
        let dirs = scratch("live_many");
        let mut editor = Editor::new();
        // Four megabytes a painting at 1024 square: 40 is more than the
        // 128 MB budget holds.
        let count = LIVE_BYTES / (1024 * 1024 * 4) + 8;
        for index in 0..count {
            editor
                .apply(Command::AddShapeClip {
                    above: false,
                    track_id: None,
                    start: index as f64,
                    style: Some(concat_project::model::ShapeStyle {
                        width: 0.1 + index as f64 * 0.001,
                        ..Default::default()
                    }),
                    duration: Some(1.0),
                    name: format!("Shape {index}"),
                })
                .expect("a shape is added");
        }
        let titles = Titles::new(&dirs);
        let first = titles.clips_live(editor.project(), 1024, 1024);
        assert_eq!(first.len(), count);
        let second = titles.clips_live(editor.project(), 1024, 1024);
        let repainted = first
            .iter()
            .zip(&second)
            .filter(|(before, after)| {
                !Arc::ptr_eq(
                    before.frame.as_ref().expect("live"),
                    after.frame.as_ref().expect("live"),
                )
            })
            .count();
        assert_eq!(repainted, 0, "of {count}");
    }

    /// A painting asked for again is the most recently used, and outlives
    /// one asked for less recently; past the ceiling the first painted
    /// stay and the newest is not kept.
    #[test]
    fn live_paintings_go_least_recently_used_first() {
        let art = || Art {
            block: (1, 1),
            offset: (0, 0),
            canvas: (4, 4),
            reveal: Arc::new(RevealMap::from_rects(1, 1, &[])),
        };
        let frame = || Arc::new(Frame::transparent(4, 4));
        let size = frame().pixels().len();
        let mut live = LivePaintings::new(2 * size, 3 * size);

        live.begin();
        live.keep(1, art(), frame());
        live.keep(2, art(), frame());
        live.begin();
        assert!(live.get(1).is_some(), "1 is used again");
        live.begin();
        live.keep(3, art(), frame());
        assert!(live.get(1).is_some(), "the recently used stays");
        assert!(live.get(2).is_none(), "the least recently used goes");

        // One pass asking for more than the budget keeps them all, to the
        // ceiling; past it, the newest is the one not kept.
        live.begin();
        for key in 10..14 {
            live.keep(key, art(), frame());
        }
        assert!(live.get(10).is_some() && live.get(11).is_some() && live.get(12).is_some());
        assert!(live.get(13).is_none());
        assert!(live.bytes <= 3 * size);
    }

    /// A shape clip rejoins the flattened list as a still at its clip's
    /// stretch - that is how a square becomes a rectangle - with the
    /// figure's box reported and no words to reveal.
    #[test]
    fn a_shape_rejoins_as_a_still_and_keeps_its_stretch() {
        let dirs = scratch("shape");
        let mut editor = Editor::new();
        let id = editor
            .apply(Command::AddShapeClip {
                above: true,
                track_id: None,
                start: 0.0,
                style: None,
                duration: Some(3.0),
                name: "Square".to_owned(),
            })
            .expect("a shape is added")
            .created_id
            .expect("with an id");
        editor
            .apply(Command::SetClipTransform {
                clip_id: id.clone(),
                scale: None,
                offset_x: None,
                offset_y: None,
                rotation: None,
                stretch_x: Some(2.0),
                stretch_y: Some(0.5),
            })
            .expect("pulled wide");

        let out = Titles::new(&dirs).clips(editor.project(), 640, 360);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].clip_id, id);
        assert!(matches!(out[0].clip.kind, ClipKind::Image));
        assert_eq!((out[0].clip.stretch_x, out[0].clip.stretch_y), (2.0, 0.5));
        assert_eq!(out[0].block, (126, 126), "35 % of 360, squared");
        assert_eq!(out[0].offset, (0, 0));
        assert!(out[0].clip.reveal_map.is_none());
        assert!(
            Path::new(&out[0].clip.path).is_file(),
            "painted to disk like a title"
        );
    }

    /// A shape's Animate moves, mask and fades rejoin with it. The title
    /// path builds its clip from blank, and left all three behind: no In,
    /// Out or Loop ever played on a title, a caption or a shape.
    #[test]
    fn a_shape_rejoins_with_its_motion_mask_and_fades() {
        use concat_project::commands::ClipPatch;
        use concat_project::model::{ClipMotion, ClipMove, Mask, MaskShape};
        let dirs = scratch("shape-motion");
        let mut editor = Editor::new();
        let id = editor
            .apply(Command::AddShapeClip {
                above: true,
                track_id: None,
                start: 0.0,
                style: None,
                duration: Some(3.0),
                name: "Square".to_owned(),
            })
            .expect("a shape is added")
            .created_id
            .expect("with an id");
        let motion = ClipMotion {
            enter: Some(ClipMove {
                kind: "pop".to_owned(),
                seconds: 0.5,
            }),
            ..ClipMotion::default()
        };
        let masks = vec![Mask::of("mask-1", "", MaskShape::Circle)];
        editor
            .apply(Command::UpdateClip {
                clip_id: id,
                patch: ClipPatch {
                    motion: Some(Some(motion.clone())),
                    masks: Some(masks.clone()),
                    fade_in: Some(0.25),
                    fade_out: Some(0.75),
                    ..ClipPatch::default()
                },
            })
            .expect("animated, masked and faded");

        let out = Titles::new(&dirs).clips(editor.project(), 640, 360);
        assert_eq!(out[0].clip.motion, Some(motion));
        assert_eq!(out[0].clip.masks, masks);
        assert_eq!((out[0].clip.fade_in, out[0].clip.fade_out), (0.25, 0.75));
    }
}

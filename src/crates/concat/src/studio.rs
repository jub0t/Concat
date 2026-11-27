// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The window's state, and how it is published into Slint.
//!
//! Two kinds of state live here and the line between them is the whole
//! design:
//!
//! - **The edit** is the engine's. It lives in the open [`Session`], every
//!   change to it is a [`Command`], and this module only ever reads it back.
//!   A gesture in flight is previewed on an *echo* - a clone of the project
//!   the pointer mutates - and committed as one command on release, so undo
//!   undoes the drag and not a pixel of it.
//! - **The view** is the window's: selection, playhead, zoom, tool, the
//!   workspace's arrangement, which lanes are locked, what the dialogs show.
//!   None of it reaches the document.
//!
//! Everything Slint draws is produced by `publish` and its two halves from
//! those two, on every event that could have changed either.

#![allow(
    clippy::type_complexity,
    clippy::collapsible_if,
    clippy::manual_unwrap_or_default
)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use concat_doc::commands::{
    ClipMove, Command, Edge as TrimEdge, Framing, ItemPatch, KeyProperty, LaneFlag,
};
use concat_doc::model::{
    self, Content, Effect, Item, ItemId, LaneId, MediaId, Model as Doc, TextAlign, Title,
};
use concat_doc::{AnyId, Document, Tick, TickDur, TickLen};
use concat_effects::Catalogue;
use concat_effects::manifest::Kind as PackageKind;
use concat_effects::manifest::ParamType;
use concat_host::playback::ClipSpec;
use concat_host::{
    AnalyseRequest, Cutouts, EnhanceRequest, ProjectInfo, RegionRequest, Session, media, projects,
    templates,
};
use concat_media::Pyramid;

use crate::doc_view::{self, Kind};

/// The item an edit made, as the panels hold its id.
fn made_item(created: Option<AnyId>) -> Option<String> {
    created
        .and_then(|id| id.typed::<ItemId>())
        .map(|id| id.to_string())
}

/// The colour the sequence showing is output in; SDR with none.
fn colour_of_showing(doc: &Doc) -> model::ColorSpace {
    doc_view::sequence(doc).map_or(model::ColorSpace::Sdr, |sequence| {
        sequence.frame.color.clone()
    })
}

mod backdrop;
mod beats;
mod consolidate;
use consolidate::MergeJob;
mod masking;
mod masks;
use masking::{MaskFrame, MaskGrip};
mod bin;
mod chains;
#[cfg(test)]
mod copy_tests;
mod edit;
mod gestures;
mod inspector;
mod keyboard;
mod keyframes;
mod menus;
#[cfg(test)]
mod merge_words_tests;
mod monitor;
mod placing;
mod project_files;
mod publish;
mod speech;
#[cfg(test)]
mod speed_curve_tests;
mod stage;
mod text;
mod toast;
pub use toast::ToastState;
mod vision;
mod voice;
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::dock::{
    Dock, DockLayout, SEAT_GAP, default_dock, lay_out, nearest_row, row_at, row_top,
};
use crate::format::{
    WAVE_BAR, WAVE_PITCH, frames_timecode, hex_of, running_time, wave_columns, wave_path,
    when_phrase,
};
use crate::grading;
use crate::host::{
    CachedStrip, Host, MediaArt, MediaPoster, WindowArt, cached_media_art, cached_window_art,
    image_at, image_of, media_peaks, media_poster, media_strip, on_ui_in_project, spawn, spawn_art,
    spawn_in_project, spawn_long, spawn_long_in_project, spawn_poster, spawn_strip, spawn_wave,
    strip_window, window_art, window_span, window_start,
};
use crate::i18n::{self, t, tf};
use crate::keymap;
use crate::panes::settings::installed;
use crate::platform;
use crate::prefs::Preferences;
use crate::presets::{self, TextPreset};
use crate::speed_ramp;
use crate::ui::*;

/// The monitor's output sizes, matching the picker's rows. 4:5 and 3:4
/// are the upright frames an Instagram post is cut to: its feed and its
/// grid. Typing 1080 x 1350 by hand was the only way to them.
pub const OUTPUTS: [(i32, i32); 8] = [
    (1920, 1080),
    (3840, 2160),
    (1080, 1920),
    (1080, 1080),
    (1080, 1350),
    (1080, 1440),
    (1440, 1080),
    (2560, 1080),
];

/// Shortest clip the editor will make: a sixtieth of a second. Trims and
/// splits both floor at this, as the engine's own `MIN_CLIP_DURATION` does.
pub const MIN_DURATION: f32 = 1.0 / 60.0;

/// The three lane heights, in logical pixels.
///
/// Picture gets the tallest because a filmstrip is the one thing that needs
/// the room; sound the middle, where an envelope still has shape. The ladder
/// is what `TrackSize::Auto` picks from - see `lane_height`. Raised from
/// 80/60/40: with the name strip and the sound band taken off, a video's
/// frames had under forty pixels and a waveform under thirty, and both
/// read as crammed.
const LANE_LARGE: f32 = 108.0;
const LANE_MEDIUM: f32 = 80.0;
const LANE_SMALL: f32 = 44.0;

/// How long a title runs when it is placed: long enough to read, short
/// enough that trimming it is a nudge rather than a fight.
const LAYER_DURATION: f32 = 3.0;
/// How long a transition clip covers when first placed: a second, half
/// either side of the cut it lands on.
const TRANSITION_DURATION: f32 = 1.0;

/// One media item's filmstrip, as the lanes tile it.
pub struct Strip {
    /// Every sampled frame side by side.
    pub image: slint::Image,
    /// How many frames the picture holds.
    pub frames: i32,
    /// One frame's width in the picture's pixels.
    pub frame_width: i32,
    /// The picture's height in its own pixels.
    pub height: i32,
}

/// One frame's middle through the Enhance model, before and after, for
/// the inspector to show before a whole file is committed to.
struct EnhancePreview {
    /// The clip it was made for, and the file and size it was made from:
    /// a preview of other settings or another file is not this one.
    clip_id: String,
    media_id: String,
    scale: f32,
    /// The plain enlargement and the model's full-strength answer, kept so
    /// Strength re-blends them at once rather than running the model again.
    plain: concat_core::Frame,
    enhanced: concat_core::Frame,
    before: slint::Image,
    after: slint::Image,
    /// The whole percent `after` is blended at.
    after_percent: u32,
    /// See `concat_host::enhance::Preview`.
    tile_seconds: f64,
    tiles_per_frame: usize,
}

/// The Enhance section's rows for one clip; see `Studio::enhance_readout`.
#[derive(Default)]
struct EnhanceReadout {
    /// The scale a run would make, once to four times: Auto's, or the ask
    /// held to what this file may take.
    scale: f32,
    /// What Auto comes to for this file, and the most it may be enlarged:
    /// as far as 2K, never past the model's four times.
    auto_scale: f32,
    max_scale: f32,
    /// The file's size and the copy's, "1280 × 720" and "2560 × 1440".
    source: String,
    output: String,
    /// Why the scale cannot move, or empty.
    size_note: String,
    /// Before, after and the estimate for a whole run - its time, its
    /// frames and each frame's time, the last two empty for a still - when
    /// a preview of these settings is on hand.
    preview: Option<(slint::Image, slint::Image, (String, String, String))>,
}

impl EnhancePreview {
    /// `after` blended at `strength`, unless it already is.
    fn blend_at(&mut self, strength: f32) {
        let percent = concat_host::enhance::strength_percent(strength);
        if percent != self.after_percent {
            self.after = image_of(&concat_vision::enhance::blend(
                &self.enhanced,
                &self.plain,
                strength,
            ));
            self.after_percent = percent;
        }
    }
}

impl From<CachedStrip> for Strip {
    fn from(strip: CachedStrip) -> Self {
        Self {
            image: strip.image,
            frames: strip.frames as i32,
            frame_width: strip.frame_width as i32,
            height: strip.height as i32,
        }
    }
}

impl Strip {
    /// A decoded strip of `frames` frames.
    fn of(frame: &concat_core::frame::Frame, frames: u32) -> Self {
        Self {
            image: image_of(frame),
            frames: frames as i32,
            frame_width: (frame.width() / frames.max(1)) as i32,
            height: frame.height() as i32,
        }
    }
}

/// The key a cell's strip is held under: the media and the cell.
fn window_key(media_id: &str, level: u32, cell: u32) -> String {
    format!("{media_id}|{level}|{cell}")
}

/// How many cell strips are kept before the ones no clip on the current
/// timeline shows are let go. Each is a picture the size of the file's own
/// strip; a trim walks through several levels on its way down.
const WINDOWS_KEPT: usize = 96;

/// Steps per second the drawn waveform is quantised to. See `Studio::wave`:
/// it is what keeps a trim from synthesising a new envelope on every pointer
/// event, and the grid is fine enough that no step of it is visible.
const WAVE_STEPS: f32 = 30.0;

/// The export dialog's ladders, matching its rows.
/// The export sheet's resolution ladder, as the short side of the frame:
/// 4K, QHD, 1080p, 720p. The long side follows the project's own aspect,
/// so a 9:16 edit exports as 1080 x 1920 and a 21:9 one as 2520 x 1080 -
/// the picker chooses how fine, never which way round.
pub const EXPORT_SHORT_SIDES: [u32; 4] = [2160, 1440, 1080, 720];
/// Narrower than this, in logical pixels, and the workspace shows the
/// compact dock: a phone, a tablet held upright, a desktop window squeezed
/// to a corner. Four panes across less than this is four slivers.
pub const COMPACT_WIDTH: f32 = 860.0;
/// The quality an export's encoder falls back on were it given no
/// bitrate: the export sheet always gives one.
pub(crate) const EXPORT_CRF: u8 = 20;
pub(crate) const AUDIO_BPS: f32 = 192_000.0;

/// The frame shapes the launch screen offers, as the ratio behind each
/// label: width over height.
///
/// A shape and a size rather than the four fixed frames this was. Those
/// offered one upright frame at one size and no square at all, so a phone
/// cut at 4K was not a thing the form could ask for. Every pair of indices
/// resolves through [`frame_size`], which is the one place that knows what
/// a label means in pixels.
///
/// 4:5 and 3:4 are an Instagram post's feed and grid frames. 21:9 is the
/// 64:27 every screen sold as 21:9 is, so it lands on the 2560 x 1080 the
/// monitor's picker calls 21:9 rather than a 2520 of its own.
pub const ASPECTS: [(&str, u32, u32); 7] = [
    ("16:9", 16, 9),
    ("9:16", 9, 16),
    ("1:1", 1, 1),
    ("4:5", 4, 5),
    ("3:4", 3, 4),
    ("4:3", 4, 3),
    ("21:9", 64, 27),
];

/// The sizes, as the *short* edge in pixels.
///
/// Which is what the "p" in 1080p has always counted, and the only reading
/// that survives turning a frame upright: 1080p landscape is 1920 x 1080
/// and 1080p vertical is 1080 x 1920, the same number of lines either way.
/// Naming the long edge instead would make a vertical 1080p a 1080 x 1920
/// frame at one moment and a 608 x 1080 frame at another.
pub const SIZES: [(&str, u32); 4] = [
    ("720p", 720),
    ("1080p", 1080),
    ("1440p", 1440),
    ("4K", 2160),
];

/// The frame an aspect and a size name, in pixels.
///
/// Both indices are clamped rather than trusted: they arrive from the form
/// as plain integers, and a frame is not worth a panic.
pub fn frame_size(aspect: usize, size: usize) -> (u32, u32) {
    let (_, w, h) = ASPECTS[aspect.min(ASPECTS.len() - 1)];
    let (_, short) = SIZES[size.min(SIZES.len() - 1)];
    // Multiplied before divided, so 1080 x 16 / 9 is 1920 and not 1080.
    let long = even(short * w.max(h) / w.min(h));
    if w >= h {
        (long, even(short))
    } else {
        (even(short), long)
    }
}

/// Rounded down to an even number of pixels.
///
/// Every ratio above already lands even at every size; this is for the next
/// one added. Chroma is subsampled by two in both directions in every format
/// the export writes, and an odd dimension is what an encoder either refuses
/// or quietly crops.
fn even(value: u32) -> u32 {
    value & !1
}

/// The frame rates a project is offered, on the launch sheet and the
/// project sheet alike, as exact fractions: 29.97 is 30000/1001 and never
/// anything else - a decimal is for reading, and export must never be
/// handed one. The rates cameras and screens run at, from a stop-motion
/// ten to a game capture's sixty; anything else is typed in as a custom
/// rate. https://github.com/jub0t/Concat/issues/199
pub const RATES: [(&str, i64, i64); 11] = [
    ("10", 10, 1),
    ("20", 20, 1),
    ("23.976", 24000, 1001),
    ("24", 24, 1),
    ("25", 25, 1),
    ("29.97", 30000, 1001),
    ("30", 30, 1),
    ("48", 48, 1),
    ("50", 50, 1),
    ("59.94", 60000, 1001),
    ("60", 60, 1),
];

/// The smallest and largest side a typed frame may have, in pixels. The
/// floor is a frame that still has a picture in it; the ceiling is 8K,
/// the texture size every GPU the compositor runs on can hold (the wgpu
/// floor), and past what any encoder the export drives will take.
pub const FRAME_SIDES: (u32, u32) = (16, 8192);

/// The slowest and fastest rate that may be typed, in frames a second.
pub const FRAME_RATES: (f64, f64) = (1.0, 240.0);

/// A typed frame as the project can hold it: each side within
/// [`FRAME_SIDES`] and even, rounded to the nearest even number rather
/// than down, so 1081 becomes 1082 and not a frame a line short.
pub fn custom_frame(width: f32, height: f32) -> (u32, u32) {
    let side = |value: f32| {
        let (low, high) = FRAME_SIDES;
        let value = if value.is_finite() { value } else { low as f32 };
        let nearest = ((value / 2.0).round() * 2.0) as i64;
        nearest.clamp(i64::from(low), i64::from(high)) as u32
    };
    (side(width), side(height))
}

/// A typed rate as the exact fraction a project stores. The NTSC rates are
/// the /1001 fractions cameras record at, so 29.97 and 23.976 come out as
/// 30000/1001 and 24000/1001 however they are rounded when typed; anything
/// else is kept to a thousandth, in lowest terms.
pub fn custom_rate(fps: f64) -> (i64, i64) {
    let (low, high) = FRAME_RATES;
    let fps = if fps.is_finite() {
        fps.clamp(low, high)
    } else {
        30.0
    };
    for whole in [24_i64, 30, 48, 60, 120, 240] {
        let ntsc = whole as f64 * 1000.0 / 1001.0;
        if (fps - ntsc).abs() < 0.006 {
            return (whole * 1000, 1001);
        }
    }
    let num = (fps * 1000.0).round() as i64;
    let divisor = gcd(num, 1000);
    (num / divisor, 1000 / divisor)
}

/// A rate as frames a second, for a field to show.
pub fn fps_of(num: i64, den: i64) -> f64 {
    num as f64 / den.max(1) as f64
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 {
        a.abs().max(1)
    } else {
        gcd(b, a % b)
    }
}

/// What one effect library is showing: the words typed into it, the shelf
/// picked, and whether the star is down.
///
/// Held here rather than in the panel because the filtering is done here:
/// the model a library renders arrives already narrowed, which is what lets
/// it be laid out as a grid - a tile's place is its index, and an index only
/// means a place when every entry in the model is one that shows.
#[derive(Clone, Debug)]
pub struct LibraryView {
    /// The search box's text.
    pub query: String,
    /// The chosen shelf, as an index into the kind's group list.
    pub group: i32,
    /// Only starred packages show, whatever shelf they are on.
    pub favourites: bool,
    /// Active category filter (e.g. "All", "Featured", "Retro & Film", etc.).
    pub category: String,
}

impl Default for LibraryView {
    /// Every shelf at once: a library opens on All, not on whichever shelf
    /// happens to be filed first.
    fn default() -> Self {
        LibraryView {
            query: String::new(),
            group: -1,
            favourites: false,
            category: String::new(),
        }
    }
}

/// Which library a shelf index names. The numbers are the panel's own -
/// `PresetStack.shelf`, and `Library.view`'s shelf argument everywhere else -
/// and the order the tabs sit in.
pub const SHELF_KINDS: [PackageKind; 4] = [
    PackageKind::Filter,
    PackageKind::Effect,
    PackageKind::Audio,
    PackageKind::Transition,
];

/// The sound packages Basic > Treatments switches on and off: a clip's
/// loudness, its noise and its voice. They are switches there and nowhere
/// a card, so the Library's sound shelf leaves them out; a sound layer,
/// which has no Treatments page, still lists them in its own chain.
pub const TREATMENTS: [&str; 3] = ["concat.loudness", "concat.denoise", "concat.enhance-voice"];

/// `name` under the home directory, as a path string; empty when there is
/// no home to speak of, and the form then asks for a folder outright.
///
/// The platform's own answer, not `$HOME`: Windows does not set that, so
/// every folder the app suggested there came up empty. `name` is written
/// with `/` and joined a part at a time, so the separators match.
pub(crate) fn home_folder(name: &str) -> String {
    std::env::home_dir()
        .filter(|home| !home.as_os_str().is_empty())
        .map(|home| {
            name.split('/')
                .fold(home, |path, part| path.join(part))
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_default()
}

/// Which edge of a clip a trim has hold of.
#[derive(Clone, Copy, PartialEq)]
pub enum Edge {
    Start,
    End,
}

/// Where one clip sat when a gesture began, so the whole set moves rigidly.
pub struct MoveOrigin {
    pub clip: String,
    pub row: i32,
}

/// A pointer gesture in flight, previewed on the echo.
pub enum Gesture {
    None,
    Move {
        /// The clip actually grabbed; it is the one that snaps.
        primary: String,
        origins: Vec<MoveOrigin>,
        /// The lane heights as they were when the press landed. Frozen, not
        /// read live: an `Auto` lane resizes as clips land on it, which
        /// would move the very edges the next event measures against.
        lanes: Vec<f32>,
        /// The pointer has left the dead zone around the press.
        started: bool,
    },
    Trim {
        clip: String,
        edge: Edge,
        start: f32,
        duration: f32,
        /// The pointer has left the dead zone around the press.
        started: bool,
    },
    /// Dragging a transition's duration handle on the timeline.
    TransitionResize {
        clip: String,
        original_duration: f32,
    },
    /// A drag on the stage: every selected picture under the playhead slides
    /// with the pointer, from where each one was when the press landed.
    StageMove {
        /// The picture grabbed; it is the one that snaps, and the rest
        /// follow it by the same amount.
        primary: String,
        origins: Vec<StageOrigin>,
        /// The press, in fractions of the frame.
        from: (f64, f64),
    },
    /// A corner grip dragged: the picture scales about its centre, by the
    /// ratio of the pointer's distance from that centre to what it was.
    StageScale {
        clip: String,
        scale: f64,
        /// The centre, in frame pixels — the unit the ratio is taken in, so
        /// a tall frame and a wide one scale at the same rate.
        centre: (f64, f64),
        from: f64,
        /// Half the picture's bounds at the press, in frame fractions, so
        /// an edge's position at any later scale is one multiplication.
        half: (f64, f64),
    },
    /// An edge grip pulls one axis: the picture's stretch across or down,
    /// about its centre, measured along the box's own axis so a turned
    /// picture stretches along itself and not along the frame.
    StageStretch {
        clip: String,
        /// The left and right edges pull the width; the others the height.
        across: bool,
        stretch: f64,
        centre: (f64, f64),
        /// The pointer's distance from the centre along the axis at the
        /// press, in frame pixels.
        from: f64,
        /// The picture's turn, to project the pointer onto its axes.
        rotation: f64,
    },
    /// A side grip on a title pulls the width its words wrap at, not the
    /// glyphs: the box edge follows the pointer and the words reflow into
    /// it. The width is twice the pointer's distance from the centre along
    /// the box's own axis, as the box is drawn about the centre.
    TextWidth {
        clip: String,
        centre: (f64, f64),
        /// The title's turn, to project the pointer onto its axis.
        rotation: f64,
    },
    /// The top or bottom grip on a title pulls the height of its box, on
    /// the same terms as [`Gesture::TextWidth`] pulls the width. Before
    /// this the two grips fell through to a stretch, and a caption pulled
    /// to fit came out with its glyphs squashed.
    /// https://github.com/jub0t/Concat/issues/119
    TextHeight {
        clip: String,
        centre: (f64, f64),
        rotation: f64,
    },
    /// The rotation grip dragged: the picture turns by the angle the pointer
    /// has swept about the centre since the press.
    StageRotate {
        clip: String,
        rotation: f64,
        centre: (f64, f64),
        from: f64,
    },
    /// A grip of the selected clip's mask dragged: the mask as the press
    /// found it, placed in the frame, and the press in frame pixels.
    StageMask {
        clip: String,
        grip: MaskGrip,
        frame: MaskFrame,
        from: (f64, f64),
    },
    /// A brush on the stage: the stroke so far, in source fractions for
    /// the command it becomes and in stage fractions for the line drawn
    /// under the pointer meanwhile.
    Paint {
        clip: String,
        tool: model::BrushTool,
        size: f64,
        /// The source instant under the playhead when the stroke began.
        at: f64,
        points: Vec<[f64; 2]>,
        screen: Vec<(f32, f32)>,
    },
}

/// The custom cutout's brushes, in the inspector's order.
pub const BRUSHES: [model::BrushTool; 4] = [
    model::BrushTool::SmartBrush,
    model::BrushTool::Brush,
    model::BrushTool::SmartEraser,
    model::BrushTool::Eraser,
];

/// A colour knob waiting for its colour off the picture: the clip it is on,
/// the link in that clip's picture chain, and the knob's key.
#[derive(Clone, Debug, PartialEq)]
pub struct ColourPick {
    pub clip: String,
    pub entry: usize,
    pub key: String,
}

/// Whether a clip of this kind has a picture of its own to take a colour
/// off: a video or a still, decoded from its file. A title or a shape is
/// drawn, and has no file to read the pixel from.
fn picks_colour(kind: Kind) -> bool {
    matches!(kind, Kind::Video | Kind::Image)
}

/// The colour about `(u, v)` of `frame`, in fractions of it: the mean of
/// the pixels within two of the one there - five by five - taken in light
/// and written back in the display encoding the frame and a colour knob
/// both hold, so a pick on grain or a codec's blocks lands on the colour
/// around it, not on one speck. None off the picture.
fn colour_about(frame: &concat_core::frame::Frame, u: f64, v: f64) -> Option<[u8; 3]> {
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return None;
    }
    let (width, height) = (frame.width() as i64, frame.height() as i64);
    if width == 0 || height == 0 {
        return None;
    }
    let x = ((u * width as f64) as i64).clamp(0, width - 1);
    let y = ((v * height as f64) as i64).clamp(0, height - 1);
    let pixels = frame.pixels();
    let mut sum = [0.0f64; 3];
    let mut count = 0.0;
    for row in (y - 2).max(0)..=(y + 2).min(height - 1) {
        for column in (x - 2).max(0)..=(x + 2).min(width - 1) {
            let at = ((row * width + column) * 4) as usize;
            let Some(pixel) = pixels.get(at..at + 3) else {
                continue;
            };
            for (total, byte) in sum.iter_mut().zip(pixel) {
                *total += (f64::from(*byte) / 255.0).powf(2.4);
            }
            count += 1.0;
        }
    }
    if count == 0.0 {
        return None;
    }
    Some(sum.map(|total| {
        ((total / count).powf(1.0 / 2.4) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    }))
}

/// Whether a colour has a channel standing clear of the other two - the
/// test a key holds a screen colour to - rather than being a grey.
fn has_hue([r, g, b]: [u8; 3]) -> bool {
    let mut channels = [r, g, b].map(|byte| (f64::from(byte) / 255.0).powf(2.4));
    channels.sort_by(f64::total_cmp);
    let [_, middle, most] = channels;
    most > 0.002 && most - middle >= 0.1 * most
}

/// Where a picture was when a stage move began.
pub struct StageOrigin {
    pub clip: String,
    pub offset_x: f64,
    pub offset_y: f64,
}

/// A picture's footprint in the output frame, in fractions of it: where the
/// centre is, how much of the frame's width and height it covers before it
/// is turned, and the clockwise turn. The one place the monitor's box and
/// the compositor's quad agree, so the outline lands on the pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Footprint {
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    pub rotation: f64,
}

impl Footprint {
    /// The half-extents of the turned box's axis-aligned bounds, in fractions
    /// of the frame: what snapping measures, since an edge of a turned
    /// picture is not a line the frame's edges can meet.
    pub fn half_bounds(&self, frame: (u32, u32)) -> (f64, f64) {
        let (width, height) = (f64::from(frame.0), f64::from(frame.1));
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        let w = self.w * width;
        let h = self.h * height;
        (
            (w * cos.abs() + h * sin.abs()) / 2.0 / width,
            (w * sin.abs() + h * cos.abs()) / 2.0 / height,
        )
    }

    /// True when the frame point `(x, y)`, in fractions, is inside the turned
    /// box. `frame` is the output size, because the box is not square in
    /// pixels and the turn has to happen in pixels.
    pub fn contains(&self, x: f64, y: f64, frame: (u32, u32)) -> bool {
        let (width, height) = (f64::from(frame.0), f64::from(frame.1));
        let dx = (x - self.cx) * width;
        let dy = (y - self.cy) * height;
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        // The inverse of the compositor's forward map: undo the clockwise
        // turn to land in the box's own frame, then it is an axis test.
        let lx = dx * cos + dy * sin;
        let ly = -dx * sin + dy * cos;
        lx.abs() <= self.w * width / 2.0 && ly.abs() <= self.h * height / 2.0
    }
}

/// A drag from the library, resolved against the timeline: the ghost the
/// lanes draw and the clip the release makes, as one answer.
#[derive(Clone)]
pub struct DropPlan {
    pub kind: ClipKind,
    pub label: String,
    /// The bin's media id, for a clip cut from a file. Empty for a title.
    pub media: String,
    pub start: f32,
    pub duration: f32,
    pub row: i32,
}

/// Every model the window is handed, kept for its lifetime rather than
/// rebuilt: a new model is a *reset*, and Slint answers a reset by dropping
/// every instance behind it - mid-gesture, that includes the TouchArea
/// holding the pointer.
pub struct Models {
    pub tabs: Rc<VecModel<TimelineTabData>>,
    pub tracks: Rc<VecModel<TrackData>>,
    pub clips: Rc<VecModel<ClipData>>,
    pub stage: Rc<VecModel<StageItemData>>,
    pub guides: Rc<VecModel<StageGuideData>>,
    pub media: Rc<VecModel<MediaItemData>>,
    pub video_effects: Rc<VecModel<EffectData>>,
    pub audio_effects: Rc<VecModel<EffectData>>,
    /// The catalogue's shelves, one list per kind, and the shelf labels.
    pub catalogue_effects: Rc<VecModel<CatalogueEntryData>>,
    pub catalogue_filters: Rc<VecModel<CatalogueEntryData>>,
    /// The looks made from tables, for the Adjustments page's LUTs shelf.
    pub catalogue_luts: Rc<VecModel<CatalogueEntryData>>,
    pub catalogue_audio: Rc<VecModel<CatalogueEntryData>>,
    /// The Library's sound-effect shelf: the same, less the treatments,
    /// which are switches under Basic > Treatments and not cards.
    pub catalogue_audio_library: Rc<VecModel<CatalogueEntryData>>,
    pub catalogue_transitions: Rc<VecModel<CatalogueEntryData>>,
    pub effect_groups: Rc<VecModel<SharedString>>,
    pub filter_groups: Rc<VecModel<SharedString>>,
    pub audio_groups: Rc<VecModel<SharedString>>,
    pub transition_groups: Rc<VecModel<SharedString>>,
    /// The selected clip's two chains and their knobs.
    pub applied_visual: Rc<VecModel<AppliedEntryData>>,
    pub applied_audio: Rc<VecModel<AppliedEntryData>>,
    pub visual_params: Rc<VecModel<AppliedParamData>>,
    /// The selected transition's knobs, kept and synced like the chains'
    /// so a row being dragged is not rebuilt under the finger.
    pub transition_knobs: Rc<VecModel<AppliedParamData>>,
    pub transition_choices: Rc<VecModel<crate::ui::TransitionChoiceData>>,
    pub audio_params: Rc<VecModel<AppliedParamData>>,
    /// The colour panel's knobs.
    pub adjust_params: Rc<VecModel<AppliedParamData>>,
    /// The colour panel's wheels and curves, and the picture's chain's by
    /// link.
    pub adjust_wheels: Rc<VecModel<WheelData>>,
    pub adjust_curves: Rc<VecModel<CurveData>>,
    pub visual_wheels: Rc<VecModel<WheelGroupData>>,
    pub visual_curves: Rc<VecModel<CurveGroupData>>,
    /// The picture's chain's colour knobs, a row each.
    pub visual_colours: Rc<VecModel<ColourKnobData>>,
    /// The picture's chain's choices, a row of segments each.
    pub visual_choices: Rc<VecModel<ChainChoiceData>>,
    /// A link's wheels and curves, and a curve's points, by the link - -1
    /// the colour panel - and the curve's key: kept like the rest, so a
    /// wheel or a point being dragged is not dropped when it is published
    /// again under the pointer.
    pub link_wheels: RefCell<HashMap<i32, Rc<VecModel<WheelData>>>>,
    pub link_curves: RefCell<HashMap<i32, Rc<VecModel<CurveData>>>>,
    pub curve_points: RefCell<HashMap<(i32, String), Rc<VecModel<CurvePointData>>>>,
    /// The Speed graph's points, kept like the rest so a point being
    /// dragged is not dropped when the graph is published under the pointer.
    pub ramp_points: Rc<VecModel<SpeedPointData>>,
    /// Which of them are chosen, one flag a point.
    pub curve_selected: Rc<VecModel<bool>>,
    /// The speed presets' names, in the interface's language.
    pub preset_labels: Rc<VecModel<SharedString>>,
    /// The keyframe cluster's rows and the libraries' views: synced like
    /// the rest, since a model handed over fresh is unequal to the last by
    /// identity and re-evaluates every binding on it.
    pub key_rows: Rc<VecModel<ClipKeyData>>,
    /// The Keyframes panel's rows; see `Studio::key_editor_rows`.
    pub key_editor_rows: Rc<VecModel<KeyRowData>>,
    /// The ruler's diamonds; see `Studio::key_marks`.
    pub key_marks: Rc<VecModel<f32>>,
    pub library_views: Rc<VecModel<LibraryViewData>>,
    pub menu: Rc<VecModel<MenuItemData>>,
    pub bar: Rc<VecModel<MenuItemData>>,
    /// The sheets' option lists: what is installed, and who can speak.
    pub caption_models: Rc<VecModel<SharedString>>,
    /// The captions sheet's languages; see `CaptionsPane::language_rows`.
    pub caption_languages: Rc<VecModel<SharedString>>,
    pub speech_models: Rc<VecModel<SharedString>>,
    pub speech_model_details: Rc<VecModel<SharedString>>,
    pub speakers: Rc<VecModel<SharedString>>,
    pub speaker_details: Rc<VecModel<SharedString>>,
    pub speaker_avatars: Rc<VecModel<crate::ui::AvatarData>>,
    pub speech_samples: Rc<VecModel<SharedString>>,
    /// The microphones the voiceover panel lists.
    pub voiceover_devices: Rc<VecModel<SharedString>>,
    pub speech_sample_waves: Rc<VecModel<SharedString>>,
    pub speech_sample_details: Rc<VecModel<SharedString>>,
    /// The Audio page's shelves.
    pub sounds_mine: Rc<VecModel<crate::ui::SoundCardData>>,
    pub voices_saved: Rc<VecModel<crate::ui::VoiceCardData>>,
    pub sounds_concat: Rc<VecModel<crate::ui::SoundCardData>>,
    pub voices_builtin: Rc<VecModel<crate::ui::VoiceCardData>>,
    pub voice_styles: Rc<VecModel<crate::ui::VoiceCardData>>,
    pub transcribers: Rc<VecModel<ModelData>>,
    pub voices: Rc<VecModel<ModelData>>,
    /// The Version page's releases, and beside each its day or that it
    /// is this one.
    pub versions: Rc<VecModel<SharedString>>,
    pub version_details: Rc<VecModel<SharedString>>,
    pub seats: Rc<VecModel<SeatBox>>,
    pub dividers: Rc<VecModel<DockDivider>>,
    pub recents: Rc<VecModel<RecentProjectData>>,
    /// The Text page's presets, published once from the loaded list.
    pub text_presets: Rc<VecModel<TextPresetData>>,
    /// The Latin ones, for the Text page's Styles shelf.
    pub style_presets: Rc<VecModel<TextPresetData>>,
    /// The rest, by the writing they are for, for its Languages shelf.
    pub language_presets: Rc<VecModel<TextPresetData>>,
    /// The families a title can be set in; see `Studio::font_families`.
    pub font_families: Rc<VecModel<SharedString>>,
    /// Those of them the font picker's search lets through.
    pub font_matches: Rc<VecModel<SharedString>>,
    /// The text presets the captions' style search lets through.
    pub caption_styles: Rc<VecModel<TextPresetData>>,
}

impl Models {
    pub fn new() -> Self {
        Self {
            tabs: Rc::new(VecModel::default()),
            tracks: Rc::new(VecModel::default()),
            clips: Rc::new(VecModel::default()),
            stage: Rc::new(VecModel::default()),
            guides: Rc::new(VecModel::default()),
            media: Rc::new(VecModel::default()),
            video_effects: Rc::new(VecModel::default()),
            audio_effects: Rc::new(VecModel::default()),
            catalogue_effects: Rc::new(VecModel::default()),
            catalogue_filters: Rc::new(VecModel::default()),
            catalogue_luts: Rc::new(VecModel::default()),
            catalogue_audio: Rc::new(VecModel::default()),
            catalogue_audio_library: Rc::new(VecModel::default()),
            catalogue_transitions: Rc::new(VecModel::default()),
            effect_groups: Rc::new(VecModel::default()),
            filter_groups: Rc::new(VecModel::default()),
            audio_groups: Rc::new(VecModel::default()),
            transition_groups: Rc::new(VecModel::default()),
            applied_visual: Rc::new(VecModel::default()),
            applied_audio: Rc::new(VecModel::default()),
            visual_params: Rc::new(VecModel::default()),
            transition_knobs: Rc::new(VecModel::default()),
            transition_choices: Rc::new(VecModel::default()),
            audio_params: Rc::new(VecModel::default()),
            adjust_params: Rc::new(VecModel::default()),
            adjust_wheels: Rc::new(VecModel::default()),
            adjust_curves: Rc::new(VecModel::default()),
            visual_wheels: Rc::new(VecModel::default()),
            visual_colours: Rc::new(VecModel::default()),
            visual_choices: Rc::new(VecModel::default()),
            visual_curves: Rc::new(VecModel::default()),
            link_wheels: RefCell::new(HashMap::new()),
            link_curves: RefCell::new(HashMap::new()),
            curve_points: RefCell::new(HashMap::new()),
            ramp_points: Rc::new(VecModel::default()),
            curve_selected: Rc::new(VecModel::default()),
            preset_labels: Rc::new(VecModel::default()),
            key_rows: Rc::new(VecModel::default()),
            key_editor_rows: Rc::new(VecModel::default()),
            key_marks: Rc::new(VecModel::default()),
            library_views: Rc::new(VecModel::default()),
            menu: Rc::new(VecModel::default()),
            bar: Rc::new(VecModel::default()),
            caption_models: Rc::new(VecModel::default()),
            caption_languages: Rc::new(VecModel::default()),
            speech_models: Rc::new(VecModel::default()),
            speech_model_details: Rc::new(VecModel::default()),
            speakers: Rc::new(VecModel::default()),
            speaker_details: Rc::new(VecModel::default()),
            speaker_avatars: Rc::new(VecModel::default()),
            speech_samples: Rc::new(VecModel::default()),
            voiceover_devices: Rc::new(VecModel::default()),
            speech_sample_waves: Rc::new(VecModel::default()),
            speech_sample_details: Rc::new(VecModel::default()),
            sounds_mine: Rc::new(VecModel::default()),
            voices_saved: Rc::new(VecModel::default()),
            sounds_concat: Rc::new(VecModel::default()),
            voices_builtin: Rc::new(VecModel::default()),
            voice_styles: Rc::new(VecModel::default()),
            transcribers: Rc::new(VecModel::default()),
            voices: Rc::new(VecModel::default()),
            versions: Rc::new(VecModel::default()),
            version_details: Rc::new(VecModel::default()),
            seats: Rc::new(VecModel::default()),
            dividers: Rc::new(VecModel::default()),
            recents: Rc::new(VecModel::default()),
            text_presets: Rc::new(VecModel::default()),
            style_presets: Rc::new(VecModel::default()),
            language_presets: Rc::new(VecModel::default()),
            font_families: Rc::new(VecModel::default()),
            font_matches: Rc::new(VecModel::default()),
            caption_styles: Rc::new(VecModel::default()),
        }
    }
}

/// Republish a list into a live model without resetting it: rows that did
/// not change are not written, and the length changes a row at a time.
pub fn sync<T: Clone + PartialEq + 'static>(model: &VecModel<T>, next: Vec<T>) {
    sync_with(model, next, |held, next| held == next);
}

/// [`sync`] with the rows compared by `same`: for a row that holds a model
/// of its own, whose `==` is the model's identity. A row rebuilt with a
/// fresh model was never equal to the one shown, so every publish wrote it
/// again - and an open dropdown whose options were swapped under it closed,
/// and a field being typed in was handed back its old value: the autosave
/// a second and a half after an edit is a publish, which is how #622 met it.
pub fn sync_with<T: Clone + 'static>(
    model: &VecModel<T>,
    next: Vec<T>,
    same: impl Fn(&T, &T) -> bool,
) {
    let mut next = next;
    let shared = model.row_count().min(next.len());
    let tail = next.split_off(shared);
    for (row, value) in next.into_iter().enumerate() {
        if !model.row_data(row).is_some_and(|held| same(&held, &value)) {
            model.set_row_data(row, value);
        }
    }
    while model.row_count() > shared {
        model.remove(model.row_count() - 1);
    }
    for value in tail {
        model.push(value);
    }
}

/// Whether two models hold the same rows, in the same order.
pub fn same_rows<T: PartialEq + 'static>(a: &ModelRc<T>, b: &ModelRc<T>) -> bool {
    a.row_count() == b.row_count()
        && (0..a.row_count()).all(|row| a.row_data(row) == b.row_data(row))
}

/// Two rows the same in everything, their models compared by their rows:
/// `with_models` copies `b`'s models into a clone of `a`, so every other
/// field - and any field added later - is held to `==` as it is.
fn same_but_models<T: Clone + PartialEq>(
    a: &T,
    b: &T,
    models_same: bool,
    with_models: impl FnOnce(&mut T, &T),
) -> bool {
    models_same && {
        let mut a = a.clone();
        with_models(&mut a, b);
        a == *b
    }
}

/// `fn $name(a, b) -> bool` for a struct whose `$field`s are models: equal
/// when those hold the same rows and everything else is `==`.
macro_rules! same_with_models {
    ($($name:ident: $ty:ty => $($field:ident),+;)+) => {$(
        pub fn $name(a: &$ty, b: &$ty) -> bool {
            same_but_models(a, b, true $(&& same_rows(&a.$field, &b.$field))+, |a, b| {
                $(a.$field = b.$field.clone();)+
            })
        }
    )+};
}

same_with_models! {
    same_chain_choice: crate::ui::ChainChoiceData => options, values;
    same_transition_choice: crate::ui::TransitionChoiceData => options, values;
    same_key_row: crate::ui::KeyRowData => keys;
    same_export: crate::ui::ExportData => resolutions, rates, rate_details;
    same_settings: crate::ui::SettingsData => shortcuts;
    same_relink: crate::ui::RelinkData => items;
    same_start: crate::ui::StartData => rate_options;
    same_threads: crate::ui::ThreadsData => frames;
    same_captions: crate::ui::CaptionsSheetData => sources;
}

/// Writes `next` through `set` unless it is `same` as what `held` shows: a
/// struct carrying a model is rebuilt with a fresh one on every publish,
/// and written regardless it reset the dialog reading it.
pub fn set_changed<T>(held: T, next: T, same: fn(&T, &T) -> bool, set: impl FnOnce(T)) {
    if !same(&held, &next) {
        set(next);
    }
}

/// Seconds of the original an enhanced copy covers beyond each end of the
/// clip it was made for, so a small trim after does not run off the copy.
const ENHANCE_MARGIN: f64 = 1.0;

/// The window's state. See the module docs for what is whose.
pub struct Studio {
    pub host: Host,
    pub prefs: Preferences,
    /// What each effect library is showing, indexed the way `SHELF_KINDS`
    /// is: 0 filters, 1 effects, 2 audio.
    pub library: [LibraryView; 4],

    // ── the edit ──
    pub session: Option<Session>,
    /// A copy of the document a gesture is changing. `doc()` reads it
    /// while it exists; a command replaces it.
    pub echo: Option<Doc>,
    /// The session's document as of its last change, taken once per change
    /// so every reader has it by reference; empty when nothing is open.
    document: Document<Doc>,
    /// Unsaved changes, and the timer that writes them.
    dirty: bool,
    autosave: slint::Timer,

    // ── the bin ──
    pub media: crate::panes::media_bin::MediaBin,
    /// Decoded art by media id, and the ids a worker is decoding for.
    pub peaks: HashMap<String, Arc<Pyramid>>,
    /// Each sound file's beats by media id, the ids being heard, and those
    /// that could not be; see `studio::beats`.
    pub beats: HashMap<String, Arc<concat_speech::beats::Beats>>,
    pub beats_pending: HashSet<String>,
    pub beats_failed: HashSet<String>,
    /// Filmstrips by media id: the picture, how many frames are in it, one
    /// frame's width and the strip's height, in the picture's own pixels.
    pub strips: HashMap<String, Strip>,
    /// Filmstrips of one cell of a file each, by `window_key`, for the cuts
    /// too short a piece of their footage for the file's strip to show as
    /// more than one frame repeated. See `host::strip_window`.
    pub windows: HashMap<String, Strip>,
    art_pending: HashSet<String>,
    /// Art keys a decode came back empty for: not asked again this
    /// project, or a file that cannot be read would be decoded on every
    /// event.
    art_failed: HashSet<String>,
    /// What the last art scan was for: the document's revision, the
    /// screen, and how much was pending. The scan runs when one of them
    /// changes and is a comparison on the pointer stream between.
    art_stamp: Option<(u64, bool, usize, bool)>,
    /// Media whose proxy has been seen to, this project: `proxy::ensure`
    /// costs two stats of the file, and once is enough.
    proxied: HashSet<String>,
    window_pending: HashSet<String>,
    /// Envelopes, keyed by the things they are computed from. A move
    /// changes none of them, and a publish happens on every frame of one.
    waves: RefCell<HashMap<String, SharedString>>,

    // ── the view ──
    /// The timeline's view: scroll, zoom, tool, and what the lanes know
    /// about a track that the document does not.
    pub lanes: crate::panes::timeline::TimelinePane,
    pub selection: Vec<String>,
    /// The media bin had the last press: Ctrl+A selects its cards (#696).
    pub media_focused: bool,
    pub playhead: f32,
    pub playing: bool,
    transport: slint::Timer,
    /// The phone's lanes carrying on after a finger throws them, and the
    /// beat that moves them while they do; see `panes::glide`.
    coast: crate::panes::glide::Coast,
    glide_beat: slint::Timer,
    /// The preview axis: the instant under the pointer while it crosses the
    /// lanes, which the monitor shows instead of the playhead's. None when
    /// the pointer is elsewhere, or the axis is off, or the cut is playing.
    pub hover: Option<f32>,
    /// Stops the burst of sound a hover with audio plays.
    hover_hush: slint::Timer,
    /// One clip, held for Paste.
    pub clipboard: Vec<Held>,
    /// The monitor: its frame, and the requests for the next.
    pub monitor: crate::panes::monitor::MonitorPane,

    // ── the sheets and menus ──
    pub export: crate::panes::export::ExportPane,
    pub settings: crate::panes::settings::SettingsPane,
    pub relink: crate::panes::relink::RelinkPane,
    /// The notice that part of the project cannot be shown, held back while
    /// the relink dialog the same open raised is up; raised when it closes.
    pub unread_pending: Option<String>,
    /// What the confirmation sheet is asking about, while it is up.
    pub confirm: Option<Confirm>,
    /// The clip whose incoming transition is selected, while no clip is:
    /// a transition is picked on its own, not as part of the clip it rides
    /// into. Read through [`Studio::selected_transition`], which holds it
    /// to an empty clip selection and a clip that still has one.
    pub transition_selected: Option<String>,
    /// The monitor has the whole window, and the window the whole screen:
    /// the full-screen preview. A fact of the sitting, not a preference -
    /// the next launch opens on the panels. Read it through
    /// [`Studio::preview_full`], which knows there is no monitor on the
    /// launch screen.
    pub preview_full: bool,
    /// What the window was last told about it, and whether it already
    /// covered the screen before the preview asked: a window its person
    /// made full-screen stays so when the preview gives the screen back.
    /// See `sync_full_screen` in lib.rs.
    pub full_screen_shown: bool,
    pub window_was_full: bool,
    pub open_menu: i32,
    pub menu_bar_token: i32,
    pub menu_target: Option<String>,
    /// The media the bin's menu was opened on, when it was the bin's menu
    /// and not a clip's: the two share one set of rows and one token.
    pub menu_media: Option<String>,
    pub menu_token: i32,
    /// The run of one kind of continuous message being folded into one
    /// line of the activity trail; see `note_activity`.
    activity_burst: Option<crate::panes::Burst>,
    pub toast: ToastState,

    // ── the launch screen ──
    pub on_start: bool,
    pub start: crate::panes::start::StartPane,
    /// The optional Threads quick-video workflow, separate from normal project creation.
    pub threads: crate::panes::threads::ThreadsPane,
    pub recents: Vec<projects::Recent>,
    pub posters: HashMap<String, slint::Image>,
    posters_pending: HashSet<String>,
    /// How long each recent project's timeline runs, in seconds, read on
    /// the poster's worker. A path here has been tried, poster and all,
    /// and is not asked for again until its project is next closed.
    lengths: HashMap<String, Option<f64>>,
    pub project_name: String,

    // ── the workspace ──
    pub dock: Dock,
    /// The dock not showing: the compact one while the window is wide,
    /// the wide one while it is compact. Swapped with `dock` as the
    /// window crosses `COMPACT_WIDTH`, so each keeps its shape.
    pub dock_aside: Dock,
    /// Whether `dock` is the compact one.
    pub compact: bool,
    /// Whether `dock` held the Text to speech pane when last looked at, so
    /// a dock change can tell the pane taken out from one never put in.
    pub speech_docked: bool,
    pub workspace: (f32, f32),
    pub divider_press: Option<(usize, f32, f32)>,
    pub gesture: Gesture,
    /// The snap lines of a stage move in flight; empty between moves.
    pub stage_guides: Vec<StageGuideData>,
    /// Where the inspector should go, and a count that changes every time
    /// something is applied from the library; see `Editor.inspector-jump-token`.
    pub inspector_jump: (i32, &'static str, &'static str),
    /// Bumped when the library should show its Media page: a file the
    /// editor just made and put in the bin. See `Editor.media-jump-token`.
    pub media_jump: i32,
    /// Bumped to turn the library to the Captions page; see
    /// `Editor.captions-jump-token`.
    pub captions_jump: i32,
    /// A look being shown before it is laid down: the filter's id. It is
    /// drawn into the frames the monitor asks for as a layer over the whole
    /// picture, and nowhere else - the timeline does not have it until the
    /// card is double-clicked or its plus pressed, which lays a filter
    /// layer at the playhead.
    audition: Option<String>,
    /// Counts every change to the document. What the flattened clip list
    /// below is keyed on, so a frame of an unchanged document reuses it.
    revision: u64,
    /// The last flattening of the document for the monitor - titles
    /// included - and the revision and output size it was made at. Shared
    /// with the monitor by pointer, so it can keep its plan for as long as
    /// the list is the same one.
    flat: Option<(
        u64,
        u32,
        u32,
        std::sync::Arc<Vec<concat_export::ExportClip>>,
    )>,
    /// A run painting the titles the monitor is missing is under way; see
    /// `Studio::paint_titles`.
    titles_painting: bool,
    /// The revision and size the painter last ran for.
    titles_tried: Option<(u64, u32, u32)>,
    /// An inspector commit waiting to land: a knob being dragged commits
    /// on every move, and each commit was a command, an undo of the last,
    /// a rebuild of the mix and a full publish. The commit is held until
    /// the moves pause; the echo shows the value meanwhile.
    commit_pending: bool,
    /// The clip the held commit is for: the one that was selected when the
    /// inspector wrote to the echo. Kept apart from the selection because
    /// the two can come apart before the commit lands - a press on the
    /// lanes' floor blurs a title's text box, which holds the commit, and
    /// then clears the selection on the release - and a commit that looked
    /// the clip up in the selection at landing time found nothing and
    /// dropped the words with the echo.
    /// The clips the echo was written for; see `commit_now`. Several when
    /// the inspector edits every selected title at once.
    commit_targets: Vec<String>,
    /// The words of the selected title that the Text tab's styling
    /// controls are aimed at - (clip, start, end) in characters of its
    /// content - while some are selected in its text box. None styles the
    /// whole title, as the tab always did. Kept when the box loses focus,
    /// since reaching for the colour swatch is what blurs it; read only
    /// while that clip is still the one selected.
    words: Option<(String, usize, usize)>,
    commit_timer: slint::Timer,
    /// Saves the layout once the panels have stopped moving; see
    /// `remember_layout_soon`.
    layout_timer: slint::Timer,
    /// What the catalogue shelves were last built from; while nothing in
    /// it changes the shelves are not rebuilt.
    shelf_stamp: std::cell::RefCell<Option<ShelfStamp>>,
    /// Every card still on the shelves, by package id, loaded once - an
    /// author's own `preview.png`, else the card drawn for the package -
    /// and kept: the shelves are rebuilt on every publish and a picture
    /// read from disk each time would be the slowest thing in the window.
    /// A card not drawn yet is kept as no picture until it arrives.
    look_art: std::cell::RefCell<HashMap<String, slint::Image>>,
    /// Where each picture package's card is kept, by package id; see
    /// `concat_host::cards`. Planned whenever the catalogue loads.
    card_paths: HashMap<String, std::path::PathBuf>,
    /// How many cards have arrived since the window opened: part of the
    /// shelf stamp, so a card that lands rebuilds the shelves.
    cards_drawn: u64,
    /// Whether cards are being drawn, and whether the catalogue changed
    /// while they were, which makes for another round when this one ends.
    cards_busy: bool,
    cards_again: bool,
    /// The watch on the user's effects folder: a poll every couple of
    /// seconds of what is in it, cheaper than a file-system watcher and
    /// one dependency fewer, at a cost nobody will notice.
    packages_watch: slint::Timer,
    /// The folder as it was when the catalogue was last built, and, once
    /// a change has been seen, the folder as it was at that sight: the
    /// catalogue is rebuilt only when two polls agree, so a package still
    /// being copied in is not loaded half-way.
    packages_seen: u64,
    packages_pending: Option<u64>,
    /// The last inspector commit: what it changed and when. A control that
    /// is dragged commits on every move, and each of those would be an undo
    /// step of its own; a commit that changes the same thing as the last
    /// one, within a moment of it, replaces it in the history instead.
    last_commit: Option<(String, std::time::Instant)>,
    /// Each title's painted block in frame pixels, by clip id, as of the
    /// last time the monitor asked for a frame. What the stage box for a
    /// text clip is drawn from; see `footprint`.
    /// Per text clip: the painted block's size in frame pixels, and its
    /// centre's offset from the clip's centre - see `TitleClip::offset`.
    #[allow(clippy::type_complexity)]
    pub title_blocks: HashMap<String, ((u32, u32), (i32, i32))>,
    pub drop: Option<DropPlan>,
    pub project_sheet: crate::panes::project::ProjectPane,
    pub captions: crate::panes::captions::CaptionsPane,
    pub speech: crate::panes::speech::SpeechPane,
    /// The library's Audio page: the user's own sounds.
    pub sounds: crate::panes::sounds::SoundsPane,
    /// A file from the library lent the monitor and the speakers; see
    /// `crate::source`.
    pub source: Option<crate::source::Source>,
    /// The VU meters' feed; see `crate::meters`.
    pub meters: crate::meters::MeterFeed,
    /// The voiceover take, while one runs.
    pub voiceover: crate::panes::voiceover::VoiceoverPane,
    /// Every speaker the voice engine offers, in its own order.
    /// The looks the Text page offers; see `presets`.
    pub text_presets: Vec<TextPreset>,
    /// The fonts in the app's own folder - imported from the Text
    /// inspector, or brought by a preset - as (family, file), read once at
    /// start and added to as files come in. Offered to every title; a
    /// project a title is set in one of them registers the file so the
    /// painter finds it. See `presets::installed_fonts`.
    pub installed_fonts: Vec<(String, String)>,
    /// The families the machine has, found on a worker after the window is
    /// up - the system's fonts take a moment to read - and empty until then.
    pub system_fonts: Vec<String>,
    /// A fingerprint of what `font_families` was last published from, so
    /// a publish that changes none of it leaves the picker's list alone.
    fonts_published: std::cell::Cell<Option<u64>>,
    /// What the font picker's search box holds.
    pub font_query: String,
    /// The families and the query the search's matches were last
    /// published from, hashed.
    font_search_published: std::cell::Cell<Option<u64>>,

    /// The languages Settings › General offers, in its order; see `i18n`.
    pub languages: Vec<i18n::Language>,

    // ── cutouts ──
    /// The brush a custom cutout paints with: a row of `BRUSHES`.
    pub brush: usize,
    /// The brush's diameter, as a fraction of the picture's width.
    pub brush_size: f64,
    /// Whether a press on the stage paints the selected custom cutout
    /// rather than moving the picture.
    pub painting: bool,
    /// The colour knob whose eyedropper is armed: the next press on the
    /// stage picks its colour off the clip's own picture, not the frame's.
    pub picking: Option<ColourPick>,
    /// The mask analyses running, by media id, and how far each has got.
    cutout_jobs: HashMap<String, (bool, f32)>,
    /// The stabilization analyses running, by media id, and how far each
    /// has got. One at a time, like the masks.
    stabilize_jobs: HashMap<String, f32>,
    /// Media whose stabilization was turned on anew, so an analysis the
    /// app died in last time is tried again rather than skipped.
    stabilize_retry: HashSet<String>,
    /// Cutout and stabilize analyses that failed, by the folder each
    /// writes: not started again by themselves - offline, every edit
    /// started the model's download again and said the same thing again -
    /// until the person turns the feature on anew or the project opens
    /// again. A changed subject or stretch is another folder, tried as new.
    failed_analyses: HashSet<std::path::PathBuf>,
    /// The mask the Mask section and the monitor work on, by id: see
    /// `masks`. None, or an id the selected clip does not have, is its
    /// first.
    mask_focus: Option<String>,
    /// Voice isolation splitting a file, by the parts' folder: whether its
    /// model is still downloading, and how far it has got. One at a time,
    /// like the analyses.
    voice_jobs: HashMap<std::path::PathBuf, (bool, f32)>,
    /// The running split's stop.
    voice_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Files whose split failed this session: not tried again by itself,
    /// only when a clip is asked anew.
    voice_failed: HashSet<std::path::PathBuf>,
    /// The Speed graph's chosen points, by index, and the clip they are
    /// on: another clip selected chooses none.
    curve_selected: (String, Vec<usize>),
    /// The curve as it was when a drag of the chosen points began: each
    /// move is measured from it, so a point held at a limit does not drift.
    speed_grabbed: Option<Vec<model::SpeedPoint>>,
    /// Enhance at work, by clip id: whether the model is still
    /// downloading, and how far along. One at a time, like every long job.
    enhance_jobs: HashMap<String, (bool, f32)>,
    /// The Enhance section's Scale, in times larger, 0 for Auto, and its
    /// Strength, 0 to 1. The window's rather than a clip's: they are what
    /// the next run is made with, carried from clip to clip the way a
    /// tool's settings are, and a copy already made is named by its own;
    /// see `concat_host::enhance::target_for`.
    enhance_scale: f32,
    enhance_strength: f32,
    /// The last preview of a frame through the model, while the clip it
    /// was made for is the one in the inspector.
    enhance_preview: Option<EnhancePreview>,
    /// A preview being made, for the clip by this id: whether the model is
    /// still downloading, and how far along.
    enhance_previewing: Option<(String, bool, f32)>,
    /// Clips whose Enhance section is ticked but that have no copy yet: the
    /// tick only turns the section on, so it needs somewhere to live until
    /// a run leaves a copy, whose link then holds it. The window's, not
    /// the project's: a tick with nothing made is not worth an undo step.
    enhance_ticked: HashSet<String>,
    /// Reverse writing a clip's backwards copy, by clip: how far along.
    reverse_jobs: HashMap<String, f32>,
    /// A selection being rendered into one clip, with its own cancel flag.
    merge_job: Option<MergeJob>,
    /// The smart stroke being read, by the same name, while one is.
    region_job: Option<String>,
    /// The last smart stroke as the stage drew it, kept on screen from the
    /// release until the model has read what was under it, so the paint
    /// does not vanish before its answer arrives.
    pending_stroke: Option<(String, f32, bool)>,
    /// Cutouts and smart strokes the app died working on, by their masks'
    /// folder: not started again by themselves, and said once a session.
    tripped_strokes: HashSet<std::path::PathBuf>,
}

// ── conversions between the document and the window ─────────────────────

/// The key a media item's art is kept under: the media id for its pictures
/// and its default stream's waveform, and the id with the stream for the
/// waveform of another stream a clip plays. Peaks and pending jobs share it.
fn art_key(media_id: &str, stream: Option<u32>) -> String {
    match stream {
        None => media_id.to_owned(),
        Some(index) => format!("{media_id}#{index}"),
    }
}

/// The key a media item's card picture is pending or failed under.
fn poster_key(media_id: &str) -> String {
    format!("{media_id}#poster")
}

/// The key a media item's own filmstrip is pending or failed under.
fn strip_key(media_id: &str) -> String {
    format!("{media_id}#strip")
}

/// What of a media item's own art is still to be decoded; see
/// `Studio::missing_art`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MissingArt {
    poster: bool,
    strip: bool,
    peaks: bool,
}

impl MissingArt {
    fn any(self) -> bool {
        self.poster || self.strip || self.peaks
    }
}

/// One audio track's row in the Audio panel's list: the name the file gave
/// it or its number, and its channel layout.
fn audio_track_label(track: &model::SoundStream, position: usize) -> String {
    let name = if track.title.is_empty() {
        tf("studio.track", &[&(position + 1)])
    } else {
        track.title.clone()
    };
    let layout = match track.channels {
        1 => t("studio.mono"),
        2 => t("studio.stereo"),
        channels => tf("studio.channels", &[&channels]),
    };
    format!("{name} · {layout}")
}

/// What an item is to the lanes, the stage and the drag chip.
fn kind_of(doc: &Doc, item: &Item) -> ClipKind {
    match doc_view::kind(doc, item) {
        Kind::Audio => ClipKind::Audio,
        Kind::Image => ClipKind::Image,
        Kind::Text => ClipKind::Text,
        Kind::Layer => layer_kind(doc_view::package(item)),
        Kind::SoundLayer => ClipKind::SoundLayer,
        Kind::Shape => ClipKind::Shape,
        // Drawn as a layer is - a span over the lanes beneath it, named for
        // its package, with no picture of its own - in a colour of its own.
        Kind::Transition => ClipKind::Transition,
        // What the window has no panel for is drawn as footage is: a span
        // with a picture and a sound of its own.
        Kind::Video | Kind::Other => ClipKind::Video,
    }
}

/// What a picture layer is to the eye, by the package its chain starts
/// with: the colour panel's own makes an adjustment layer, an effect
/// package an effect layer, and anything else - a look, a LUT, a chain
/// emptied - a filter layer. The lanes, the drop ghost and the drag chip
/// each paint the three in their own colour.
fn layer_kind(package: Option<&str>) -> ClipKind {
    match package {
        Some(ADJUST_ID) => ClipKind::Adjustment,
        Some(id)
            if Catalogue::builtin()
                .get(id)
                .is_some_and(|package| package.kind() == PackageKind::Effect) =>
        {
            ClipKind::Effect
        }
        _ => ClipKind::Filter,
    }
}

/// How a title's lines sit, as the panels say it; centred for an
/// alignment this build does not know.
fn align_of(align: &TextAlign) -> TextAlignment {
    match align {
        TextAlign::Left => TextAlignment::Left,
        TextAlign::Center | TextAlign::Unknown(_) => TextAlignment::Center,
        TextAlign::Right => TextAlignment::Right,
    }
}

/// The character a byte offset falls at in `text`, as a text box counts
/// its caret: bytes, where the document's runs count characters. An
/// offset inside a character counts that character; one past the end or
/// below zero is held to the text.
fn char_at(text: &str, byte: i32) -> usize {
    let byte = usize::try_from(byte).unwrap_or(0);
    text.char_indices().take_while(|(at, _)| *at < byte).count()
}

/// A title as it is first placed: the embedded face, centred, white.
fn new_title_style(prefs: &crate::prefs::Preferences) -> Title {
    // The look a title was kept as, when one was (see
    // `Studio::use_title_style_for_new`); the bundled face otherwise.
    let base = prefs.title_look.as_ref().map_or_else(
        || Title {
            font_family: "Hanken Grotesk".to_owned(),
            font_weight: 600.0,
            ..Title::default()
        },
        crate::prefs::TitleLook::to_title,
    );
    Title {
        content: t("studio.newTitle"),
        runs: Vec::new(),
        ..base
    }
}

/// The cards that ship, drawn on a real GPU by scripts/bundle-cards.sh.
mod bundled_cards {
    include!(concat!(env!("OUT_DIR"), "/bundled_cards.rs"));
}

/// The card `id` shipped with, decoded: what a shelf shows until this
/// machine has drawn its own. That is never on a software adapter, which
/// draws no cards, and minutes on a Windows GPU whose shaders compile
/// through FXC, which spends them on a blur's loops while every card after
/// it waits - a shelf of sparkles (#416).
fn bundled_card(id: &str) -> Option<slint::Image> {
    let (_, bytes) = bundled_cards::CARDS.iter().find(|(card, _)| *card == id)?;
    let picture = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
        .ok()?
        .into_rgba8();
    Some(slint::Image::from_rgba8(
        slint::SharedPixelBuffer::clone_from_slice(
            picture.as_raw(),
            picture.width(),
            picture.height(),
        ),
    ))
}

/// The card a package shows on a shelf: an author's own still from the
/// package's folder, else the card drawn for it by its own shader, else the
/// one it shipped with; read once and kept.
fn package_art(
    package: &concat_effects::catalogue::Package,
    look_art: &std::cell::RefCell<HashMap<String, slint::Image>>,
    card_paths: &HashMap<String, std::path::PathBuf>,
) -> slint::Image {
    let id = package.id();
    look_art
        .borrow_mut()
        .entry(id.to_owned())
        .or_insert_with(|| {
            package
                .folder
                .as_deref()
                .and_then(|folder| image_at(&folder.join("preview.png")))
                .or_else(|| image_at(card_paths.get(id)?))
                .or_else(|| bundled_card(id))
                .unwrap_or_default()
        })
        .clone()
}

/// Whether two project paths name one folder: the same string, or the same
/// folder once both are resolved, so a path spelled another way is not
/// mistaken for another project.
fn same_folder(a: &str, b: &str) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(a), Ok(b)) if a == b
        )
}

/// A look made from a table - an imported `.cube`, or any package that
/// carries a `[lut]` - which the Adjustments page's LUTs shelf lists and
/// the Filters page does not.
fn is_lut(package: &concat_effects::catalogue::Package) -> bool {
    package.kind() == PackageKind::Filter && package.lut().is_some()
}

/// One kind's shelves for the inspector: the shelf labels, in catalogue
/// order, and every package on them with the index of its shelf. Only the
/// packages `keep` takes are shelved at all.
fn shelves(
    kind: PackageKind,
    view: &LibraryView,
    favourites: &[String],
    look_art: &std::cell::RefCell<HashMap<String, slint::Image>>,
    card_paths: &HashMap<String, std::path::PathBuf>,
    keep: impl Fn(&concat_effects::catalogue::Package) -> bool,
) -> (Vec<SharedString>, Vec<CatalogueEntryData>) {
    let mut groups: Vec<String> = Vec::new();
    let mut entries = Vec::new();
    let query = view.query.trim().to_lowercase();
    for package in Catalogue::builtin().of_kind(kind) {
        let meta = &package.manifest.effect;
        // The colour panel's own package: every picture clip can carry it,
        // and it has a tab, not a shelf.
        if meta.id == ADJUST_ID || !keep(package) {
            continue;
        }
        let category = if meta.category.is_empty() {
            "Other".to_owned()
        } else {
            meta.category.clone()
        };
        // Shelf and package names come from the manifests in English and
        // are looked up like any other string, so a locale can carry them.
        let group = match groups.iter().position(|held| *held == category) {
            Some(index) => index,
            None => {
                groups.push(category.clone());
                groups.len() - 1
            }
        };
        // The shelves are built from every package and only the entries are
        // narrowed: a strip that lost a segment because a search matched
        // nothing on it would move under the pointer as you typed.
        let name = meta.name.clone();
        let description = meta.description.clone();
        let starred = favourites.iter().any(|held| held == &meta.id);
        let shown = if view.favourites {
            starred
        } else if !query.is_empty() {
            // Across every shelf while a query is live. Someone typing
            // "echo" wants the echo, not the echo that happens to be filed
            // where they were already looking.
            name.to_lowercase().contains(&query) || description.to_lowercase().contains(&query)
        } else if !view.category.is_empty() {
            package.matches_category(&view.category)
        } else if view.group >= 0 {
            group as i32 == view.group
        } else {
            true
        };
        if !shown {
            continue;
        }
        let art = package_art(package, look_art, card_paths);
        entries.push(CatalogueEntryData {
            id: meta.id.as_str().into(),
            name: name.into(),
            category: category.as_str().into(),
            group: group as i32,
            description: description.into(),
            favourite: starred,
            art,
        });
    }
    (
        groups.into_iter().map(SharedString::from).collect(),
        entries,
    )
}

/// How a manifest's unit is read out. Anything the inspector has no words
/// for is a plain number.
fn format_of(unit: &str) -> ParamFormat {
    match unit.trim() {
        "%" => ParamFormat::Percent,
        "dB" => ParamFormat::Decibels,
        "Hz" => ParamFormat::Hertz,
        "s" => ParamFormat::Seconds,
        "ms" => ParamFormat::Millis,
        "st" => ParamFormat::Pitch,
        "K" => ParamFormat::Kelvin,
        "EV" => ParamFormat::Stops,
        "°" => ParamFormat::Degrees,
        "x" => ParamFormat::Rate,
        _ => ParamFormat::Plain,
    }
}

/// What a batch of inspector commands touches, as a string two commits can
/// be compared by: the variant names, and for a patch the fields it sets.
fn commit_key(commands: &[Command]) -> String {
    commands
        .iter()
        .map(|command| match command {
            Command::SetClipTransform { .. } => "transform".to_owned(),
            Command::SetClipCutout { .. } => "cutout".to_owned(),
            Command::SetClipSpeed { .. } => "speed".to_owned(),
            Command::SetClipSpeedCurve { .. } => "curve".to_owned(),
            Command::UpdateClip { patch, .. } => {
                let mut fields = Vec::new();
                if patch.name.is_some() {
                    fields.push("name");
                }
                if patch.volume.is_some() {
                    fields.push("volume");
                }
                if patch.fade_in.is_some() {
                    fields.push("fade_in");
                }
                if patch.fade_out.is_some() {
                    fields.push("fade_out");
                }
                if patch.opacity.is_some() {
                    fields.push("opacity");
                }
                if patch.title.is_some() {
                    fields.push("title");
                }
                if patch.effects.is_some() {
                    fields.push("effects");
                }
                if patch.filters.is_some() {
                    fields.push("filters");
                }
                if patch.crop.is_some() {
                    fields.push("crop");
                }
                format!("patch:{}", fields.join("+"))
            }
            _ => "other".to_owned(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The field a keyable property is set through - the inverse of
/// `Studio::key_property_of`, for handing a row back to the panel.
fn key_field_of(property: KeyProperty) -> ClipField {
    match property {
        KeyProperty::Scale => ClipField::Scale,
        KeyProperty::OffsetX => ClipField::OffsetX,
        KeyProperty::OffsetY => ClipField::OffsetY,
        KeyProperty::Rotation => ClipField::Rotation,
        KeyProperty::StretchX => ClipField::StretchX,
        KeyProperty::StretchY => ClipField::StretchY,
        KeyProperty::Opacity => ClipField::Opacity,
        KeyProperty::Volume => ClipField::Volume,
    }
}

/// What the catalogue shelves are a function of; see `Studio::shelf_stamp`.
#[derive(PartialEq)]
struct ShelfStamp {
    catalogue: usize,
    cards: u64,
    lang: String,
    views: Vec<(String, i32, bool, String)>,
    favourites: Vec<String>,
}

/// The built-in colour package's id; see `adjust_rows` and `Studio::adjust_set`.
const ADJUST_ID: &str = "concat.adjust";

/// The kinds the colour panel grades: a picture, and a layer, which grades
/// everything beneath it - an adjustment layer is a layer whose chain
/// starts with the panel's own package.
fn graded(kind: Kind) -> bool {
    kind.is_visual() || kind == Kind::Layer
}

/// What a clip is to the inspector: the clips of a batch share one only
/// when they share this. See [`Studio::edit_targets`].
fn inspector_family(kind: Kind) -> u8 {
    match kind {
        Kind::Text => 0,
        Kind::Video | Kind::Image => 1,
        Kind::Shape => 2,
        Kind::Layer => 3,
        Kind::Audio | Kind::SoundLayer => 4,
        Kind::Transition => 5,
        Kind::Other => 6,
    }
}

/// Where link `index` of `chain` is in another clip's `peer` chain: the same
/// package at the same turn of it, so the second blur of the clip shown is
/// the second blur of each other clip of a batch. None when that clip has
/// no such link, and the edit passes it by.
fn peer_link(chain: &[&Effect], index: usize, peer: &[&Effect]) -> Option<usize> {
    let package = &chain.get(index)?.package;
    let turn = chain[..index]
        .iter()
        .filter(|entry| &entry.package == package)
        .count();
    peer.iter()
        .enumerate()
        .filter(|(_, entry)| &entry.package == package)
        .nth(turn)
        .map(|(at, _)| at)
}

/// What a keyable property is worth on screen at a content tick: its
/// value, or once it carries keys, what they make there.
fn shown(clip: &Item, property: KeyProperty, at: Tick) -> f64 {
    doc_view::value_at(doc_view::animated(clip, property), at)
}

/// A keyable property's value in the knob's units: the model's, except
/// the level, which the knob shows in decibels and the clip holds as gain.
fn knob_value(property: KeyProperty, value: f64) -> f64 {
    match property {
        KeyProperty::Volume => {
            if value <= 0.001 {
                -60.0
            } else {
                (20.0 * value.log10()).clamp(-60.0, 24.0)
            }
        }
        _ => value,
    }
}

/// The reverse of [`knob_value`]: a knob's number as the model holds it.
fn model_value(property: KeyProperty, knob: f64) -> f64 {
    match property {
        KeyProperty::Volume => {
            if knob <= -60.0 {
                0.0
            } else {
                10f64.powf(knob / 20.0)
            }
        }
        _ => knob,
    }
}

/// The range a keyable knob runs over, in the knob's units: what the
/// curve view's height spans. The same numbers as the inspector's rows.
fn knob_range(property: KeyProperty) -> (f64, f64) {
    match property {
        KeyProperty::Scale => (0.05, 8.0),
        KeyProperty::OffsetX | KeyProperty::OffsetY => (-1.0, 1.0),
        KeyProperty::Rotation => (-180.0, 180.0),
        KeyProperty::StretchX | KeyProperty::StretchY => (0.1, 4.0),
        KeyProperty::Opacity => (0.0, 1.0),
        KeyProperty::Volume => (-60.0, 24.0),
    }
}

/// The keyable property a batch edits relative to the first clip, for the
/// fields that place a picture; None for every field a batch sets outright.
fn relative_property(field: ClipField) -> Option<KeyProperty> {
    match field {
        ClipField::OffsetX => Some(KeyProperty::OffsetX),
        ClipField::OffsetY => Some(KeyProperty::OffsetY),
        ClipField::Rotation => Some(KeyProperty::Rotation),
        ClipField::Scale => Some(KeyProperty::Scale),
        _ => None,
    }
}

/// What a batch's knob means for one of its clips. The knob shows the
/// first clip's value, `reference`, and the others keep their distance
/// from it: a position or a turn moves by the amount the knob moved, a
/// scale by the factor, so a row of titles slides together rather than
/// landing on one spot. The first clip itself lands on the knob's value.
fn batch_value(field: ClipField, value: f64, reference: f64, current: f64) -> f64 {
    match field {
        ClipField::OffsetX | ClipField::OffsetY | ClipField::Rotation => {
            current + (value - reference)
        }
        ClipField::Scale if reference > 0.0 => current * (value / reference),
        _ => value,
    }
}

/// Writes a keyable property the way the person means it: the value while
/// the property has no keys, and once it rides, the key at the content
/// tick `at` - put on if there is none there, with the ease of the key
/// behind it. A number that rides is drawn from its keys alone, so a knob
/// or a stage drag that wrote its value would change nothing on screen.
fn write_keyable(clip: &mut Item, property: KeyProperty, value: f64, at: Tick) {
    doc_view::write_at(doc_view::animated_mut(clip, property), value, at);
}

thread_local! {
    /// How the person asked for the edits being applied, when whoever is
    /// applying them knows; see [`Asked`].
    static ASKED: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

/// What [`Asked::told`] leaves in place of a way of asking: the caller has
/// written the line itself.
const TOLD: &str = "";

/// How the person asked for the edits applied while this is held - a
/// shortcut, a menu row - so the log's line for an edit says what happened
/// and how it was asked for in one: `edit: Split at 12.40 s (shortcut)`.
/// The first to say wins: a key that goes through the menu's handler is a
/// shortcut, not a menu row.
pub(crate) struct Asked(Option<&'static str>);

impl Asked {
    /// Until this is dropped, edits were asked for by `how`.
    pub(crate) fn by(how: &'static str) -> Asked {
        let before = ASKED.get();
        if before.is_none() {
            ASKED.set(Some(how));
        }
        Asked(before)
    }

    /// Until this is dropped, the caller says what happened in a line of
    /// its own - "captions: placed 120 captions" - and the edits behind it
    /// are detail.
    pub(crate) fn told() -> Asked {
        Asked(ASKED.replace(Some(TOLD)))
    }

    /// `line`, written at the level and with the ending the way of asking
    /// gives it.
    pub(crate) fn log(line: std::fmt::Arguments<'_>) {
        match ASKED.get() {
            Some(TOLD) => log::debug!("{line}"),
            Some(how) => log::info!("{line} ({how})"),
            None => log::info!("{line}"),
        }
    }
}

impl Drop for Asked {
    fn drop(&mut self) {
        ASKED.set(self.0);
    }
}

/// `count` clips, as a person says it.
fn clips_of(count: usize) -> String {
    if count == 1 {
        "1 clip".to_owned()
    } else {
        format!("{count} clips")
    }
}

/// A file's own name out of its path, which is all a line needs of it.
pub(crate) fn file_name_of(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// A command's name as the log has it: the verb the interface uses, in
/// English whatever language the interface is in. A table rather than the
/// enum's own name, so a line reads `Split`, not `SplitClips`.
fn command_kind(command: &Command) -> &'static str {
    match command {
        Command::AddMedia { .. } => "Import media",
        Command::RemoveMedia { .. } => "Remove media",
        Command::SetMediaPlaceholder { .. } => "Set media as a template slot",
        Command::FillSlot { .. } => "Fill template slot",
        Command::Batch { .. } => "Several edits",
        Command::AddClip { .. }
        | Command::AddClipAtFirstFree { .. }
        | Command::AddClipsInSequence { .. } => "Add to timeline",
        Command::AddTextClip { .. } => "Add text",
        Command::AddShapeClip { .. } => "Add shape",
        Command::AddLayerClip { sound: true, .. } => "Add audio effect layer",
        Command::AddLayerClip { .. } => "Add effect layer",
        Command::AddTransitionClip { .. } => "Add transition",
        Command::SetClipSpeedCurve { curve: Some(_), .. } => "Speed ramp",
        Command::SetClipSpeedCurve { .. } => "Remove speed ramp",
        Command::SetClipKey { .. } | Command::SetEffectKey { .. } | Command::SetMaskKey { .. } => {
            "Set keyframe"
        }
        Command::ClearClipKey { .. }
        | Command::ClearEffectKey { .. }
        | Command::ClearMaskKey { .. } => "Remove keyframe",
        Command::ClearClipKeys { .. }
        | Command::ClearEffectKeys { .. }
        | Command::ClearMaskKeys { .. } => "Remove keyframes",
        Command::SetClipCutout {
            cutout: Some(_), ..
        } => "Remove background",
        Command::SetClipCutout { .. } => "Remove background off",
        Command::SetClipStabilize {
            stabilize: Some(_), ..
        } => "Stabilize",
        Command::SetClipStabilize { .. } => "Stabilize off",
        Command::SetClipVoice { voice: Some(_), .. } => "Voice isolation",
        Command::SetClipVoice { .. } => "Voice isolation off",
        Command::AddCutoutStroke { .. } => "Smart brush stroke",
        Command::MoveClips { .. } => "Move",
        Command::MoveClipsToNewTrack { .. } => "Move to a new lane",
        Command::TrimClip { .. } => "Trim",
        Command::SplitClips { .. } => "Split",
        Command::DuplicateClips { .. } => "Duplicate",
        Command::ReplaceClipMedia { .. } => "Replace clip's file",
        Command::SetClipEnhanced { on: true, .. } => "Enhance on",
        Command::SetClipEnhanced { .. } => "Enhance off",
        Command::FreezeFrame { .. } => "Freeze frame",
        Command::MergeClips { .. } => "Merge",
        Command::ConsolidateClips { .. } => "Merge into one clip",
        Command::RemoveClips { ripple: true, .. } => "Ripple delete",
        Command::RemoveClips { .. } => "Delete",
        Command::UpdateClip { .. } => "Change clip",
        Command::StyleTextRange { .. } => "Style text",
        Command::ClearTextStyles { .. } => "Clear text styles",
        Command::SetClipSpeed { .. } => "Speed",
        Command::SetClipTransform { .. } => "Transform",
        Command::FrameClip {
            framing: Framing::Fit,
            ..
        } => "Fit to frame",
        Command::FrameClip { .. } => "Fill frame",
        Command::DetachAudio { .. } => "Detach audio",
        Command::ReattachAudio { .. } => "Reattach audio",
        Command::AddTrack { .. } => "Add track",
        Command::RemoveTrack { .. } => "Delete track",
        Command::SetTrackFlag {
            flag: LaneFlag::Visible,
            value,
            ..
        } => {
            if *value {
                "Show track"
            } else {
                "Hide track"
            }
        }
        Command::SetTrackFlag { value, .. } => {
            if *value {
                "Mute track"
            } else {
                "Unmute track"
            }
        }
        Command::PlaceCaptions { .. } => "Add captions",
        Command::SetCaptionLook { look: Some(_), .. } => "Caption style",
        Command::SetCaptionLook { look: None, .. } => "Captions to titles",
        Command::SetCover { cover: Some(_) } => "Set cover",
        Command::SetCover { cover: None } => "Cover from the edit",
        Command::AddTimeline => "New timeline",
        Command::RemoveTimeline { .. } => "Delete timeline",
        Command::SetTimelineVideo { .. } => "Project settings",
        Command::SetTimelineBackground { .. } => "Canvas background",
        Command::SetTimelineLinkage { on: true, .. } => "Linkage on",
        Command::SetTimelineLinkage { .. } => "Linkage off",
        Command::RenameTimeline { .. } => "Rename timeline",
        Command::SelectTimeline { .. } => "Switch timeline",
        Command::MoveTimeline { .. } => "Reorder timelines",
        Command::AddFont { .. } => "Add font",
        Command::RemoveFont { .. } => "Remove font",
        Command::UpdateMediaPath { .. } => "Relink media",
        Command::SetMediaColorRange { .. } => "Colour range",
        Command::SetMediaBeats { marks: None, .. } => "Beats off",
        Command::SetMediaBeats { .. } => "Beats",
    }
}

/// What a patch changes about a clip, by name. Never to what: a patch
/// carries a title's words and a clip's name.
fn patch_words(patch: &ItemPatch) -> String {
    let changed = [
        (patch.name.is_some(), "name"),
        (patch.volume.is_some(), "volume"),
        (patch.fade_in.is_some() || patch.fade_out.is_some(), "fade"),
        (patch.opacity.is_some(), "opacity"),
        (patch.keep_pitch.is_some(), "keep pitch"),
        (patch.muted.is_some(), "mute"),
        (patch.flip_h.is_some() || patch.flip_v.is_some(), "flip"),
        (patch.blend.is_some(), "blend mode"),
        (patch.crop.is_some(), "crop"),
        (patch.masks.is_some(), "masks"),
        (patch.motion.is_some(), "animation"),
        (patch.filters.is_some(), "audio effects"),
        (patch.effects.is_some(), "effects"),
        (patch.cut_in.is_some(), "transition"),
        (patch.title.is_some(), "text"),
        (patch.shape.is_some(), "shape"),
        (patch.stream.is_some(), "audio track"),
        (patch.background.is_some(), "background"),
    ];
    let names: Vec<&str> = changed
        .into_iter()
        .filter_map(|(set, name)| set.then_some(name))
        .collect();
    if names.is_empty() {
        "nothing".to_owned()
    } else {
        names.join(", ")
    }
}

/// A command as the activity trail says it: its name and the numbers that
/// place it - where, how many, how much - and for a batch the kinds inside
/// it, counted. Never its words: a title's text, a clip's or a timeline's
/// name stay out of the log.
fn command_words(command: &Command) -> String {
    let kind = command_kind(command);
    let secs = |at: &Tick| doc_view::secs(*at);
    match command {
        Command::Batch { commands } => match commands.as_slice() {
            [] => "Nothing".to_owned(),
            [only] => command_words(only),
            _ => {
                // The kinds in the order they first come, each with how
                // many: a hundred captions are one "Add text ×100".
                let mut kinds: Vec<(&'static str, usize)> = Vec::new();
                for inner in commands {
                    let name = command_kind(inner);
                    match kinds.iter_mut().find(|(known, _)| *known == name) {
                        Some((_, count)) => *count += 1,
                        None => kinds.push((name, 1)),
                    }
                }
                let more = kinds.len().saturating_sub(4);
                let mut words: Vec<String> = kinds
                    .into_iter()
                    .take(4)
                    .map(|(name, count)| {
                        if count == 1 {
                            name.to_owned()
                        } else {
                            format!("{name} ×{count}")
                        }
                    })
                    .collect();
                if more > 0 {
                    words.push(format!("{more} more kinds"));
                }
                words.join(", ")
            }
        },
        Command::AddMedia { media: file } | Command::FillSlot { file, .. } => {
            format!("{kind} {}", file_name_of(&file.path))
        }
        Command::AddClip { start, ripple, .. } => {
            let how = if *ripple {
                ", pushing what follows"
            } else {
                ""
            };
            format!("{kind} at {:.2} s{how}", secs(start))
        }
        Command::AddClipAtFirstFree { start, .. }
        | Command::AddTextClip { start, .. }
        | Command::AddShapeClip { start, .. } => format!("{kind} at {:.2} s", secs(start)),
        Command::AddClipsInSequence { media, start, .. } => {
            format!("{kind} {} at {:.2} s", clips_of(media.len()), secs(start))
        }
        Command::AddLayerClip { start, package, .. }
        | Command::AddTransitionClip { start, package, .. } => {
            format!("{kind} {package} at {:.2} s", secs(start))
        }
        Command::SetClipSpeedCurve {
            curve: Some(points),
            ..
        } => format!("{kind} with {} points", points.len()),
        // A key's place is on its clip's own content time.
        Command::SetClipKey { at, .. }
        | Command::SetEffectKey { at, .. }
        | Command::SetMaskKey { at, .. }
        | Command::ClearClipKey { at, .. }
        | Command::ClearEffectKey { at, .. }
        | Command::ClearMaskKey { at, .. } => {
            format!("{kind} {:.2} s into the clip's content", secs(at))
        }
        Command::MoveClips { moves } | Command::MoveClipsToNewTrack { moves, .. } => {
            format!("{kind} {}", clips_of(moves.len()))
        }
        Command::TrimClip {
            edge,
            delta,
            ripple,
            ..
        } => {
            let edge = match edge {
                TrimEdge::Head => "start",
                TrimEdge::Tail => "end",
            };
            let how = if *ripple { ", rippling" } else { "" };
            format!(
                "{kind} a clip's {edge} by {:+.2} s{how}",
                doc_view::dur_secs(*delta)
            )
        }
        Command::SplitClips { items, at } => {
            if items.len() == 1 {
                format!("{kind} at {:.2} s", secs(at))
            } else {
                format!("{kind} {} at {:.2} s", clips_of(items.len()), secs(at))
            }
        }
        Command::ReplaceClipMedia { file, .. } | Command::ConsolidateClips { file, .. } => {
            format!("{kind}, now {}", file_name_of(&file.path))
        }
        Command::FreezeFrame { at, .. } => format!("{kind} at {:.2} s", secs(at)),
        Command::MergeClips { items }
        | Command::RemoveClips { items, .. }
        | Command::DuplicateClips { items } => {
            format!("{kind} {}", clips_of(items.len()))
        }
        Command::UpdateClip { patch, .. } => format!("{kind}: {}", patch_words(patch)),
        Command::SetClipSpeed { speed, .. } => format!("{kind} {:.2}x", speed.display()),
        Command::SetTimelineVideo { frame, .. } => format!(
            "{kind} {}x{} {:.3} fps",
            frame.width,
            frame.height,
            frame.rate.display_per_second()
        ),
        Command::AddFont { path, .. } => format!("{kind} {}", file_name_of(path)),
        Command::RemoveFont { family } => format!("{kind} {family}"),
        Command::UpdateMediaPath { path, .. } => {
            format!("{kind} to {}", file_name_of(path))
        }
        _ => kind.to_owned(),
    }
}

/// The commands that turn `before` into `after`, the item under `item` as
/// the echo left it against the document's: a transform, a cutout, a
/// stabilization, a voice mix, a speed, the keys, and one patch for
/// everything else that differs. Empty when nothing does. A speed change
/// on a magnetic timeline moves the lane behind the clip with its end.
fn clip_edit_commands(item: ItemId, before: &Item, after: &Item, magnetic: bool) -> Vec<Command> {
    let mut commands = Vec::new();
    let (was, now) = (&before.picture, &after.picture);
    if now.scale.value != was.scale.value
        || now.offset_x.value != was.offset_x.value
        || now.offset_y.value != was.offset_y.value
        || now.rotation.value != was.rotation.value
        || now.stretch_x.value != was.stretch_x.value
        || now.stretch_y.value != was.stretch_y.value
    {
        commands.push(Command::SetClipTransform {
            item,
            scale: Some(now.scale.value),
            offset_x: Some(now.offset_x.value),
            offset_y: Some(now.offset_y.value),
            rotation: Some(now.rotation.value),
            stretch_x: Some(now.stretch_x.value),
            stretch_y: Some(now.stretch_y.value),
        });
    }
    if now.cutout != was.cutout {
        commands.push(Command::SetClipCutout {
            item,
            cutout: now.cutout.clone(),
        });
    }
    if now.stabilize != was.stabilize {
        commands.push(Command::SetClipStabilize {
            item,
            stabilize: now.stabilize.clone(),
        });
    }
    // The inspector's Voice isolation is written to the echo like the
    // rest, and lands as a command of its own.
    if after.sound.voice != before.sound.voice {
        commands.push(Command::SetClipVoice {
            item,
            voice: after.sound.voice.clone(),
        });
    }
    // A curve the person shaped lands as the curve, which brings the pace
    // and the length with it; a constant pace that moved lands as the
    // pace, and so does taking a curve away, at the pace it averaged.
    commands.extend(chains::speed_commands(item, before, after, magnetic));
    let mut patch = ItemPatch::default();
    if after.name != before.name {
        patch.name = Some(after.name.clone());
    }
    if after.sound.volume.value != before.sound.volume.value {
        patch.volume = Some(after.sound.volume.value);
    }
    if after.sound.fade_in != before.sound.fade_in {
        patch.fade_in = Some(after.sound.fade_in);
    }
    if after.sound.fade_out != before.sound.fade_out {
        patch.fade_out = Some(after.sound.fade_out);
    }
    if now.opacity.value != was.opacity.value {
        patch.opacity = Some(now.opacity.value);
    }
    if after.sound.keep_pitch != before.sound.keep_pitch {
        patch.keep_pitch = Some(after.sound.keep_pitch);
    }
    if let (Content::Media(cut), Content::Media(old)) = (&after.content, &before.content)
        && cut.stream != old.stream
    {
        patch.stream = Some(cut.stream);
    }
    if now.flip_h != was.flip_h {
        patch.flip_h = Some(now.flip_h);
    }
    if now.flip_v != was.flip_v {
        patch.flip_v = Some(now.flip_v);
    }
    if now.blend != was.blend {
        patch.blend = Some(now.blend.clone());
    }
    if now.crop != was.crop {
        patch.crop = Some(now.crop);
    }
    if after.masks != before.masks {
        patch.masks = Some(doc_view::masks_owned(&after.masks));
    }
    if now.motion != was.motion {
        patch.motion = Some(now.motion.clone());
    }
    if let Some(shape) = doc_view::shape(after)
        && doc_view::shape(before) != Some(shape)
    {
        patch.shape = Some(shape.clone());
    }
    commands.extend(key_commands(item, before, after));
    if let Some(title) = doc_view::title(after)
        && doc_view::title(before) != Some(title)
    {
        patch.title = Some(title.clone());
    }
    if after.effects != before.effects {
        patch.effects = Some(doc_view::chain_owned(&after.effects));
    }
    if after.filters != before.filters {
        patch.filters = Some(doc_view::chain_owned(&after.filters));
    }
    if patch != ItemPatch::default() {
        commands.push(Command::UpdateClip { item, patch });
    }
    commands
}

/// The commands that turn `before`'s keys into `after`'s: a `SetClipKey`
/// for every key added or changed, a `ClearClipKey` for every key gone.
/// Two keys within a hair of each other are the same key.
fn key_commands(item: ItemId, before: &Item, after: &Item) -> Vec<Command> {
    let mut commands = Vec::new();
    for property in doc_view::KEY_PROPERTIES {
        let was = &doc_view::animated(before, property).keys;
        let now = &doc_view::animated(after, property).keys;
        for key in now {
            let unchanged = was.iter().any(|old| {
                doc_view::near(old.at, key.at) && old.value == key.value && old.ease == key.ease
            });
            if !unchanged {
                commands.push(Command::SetClipKey {
                    item,
                    property,
                    at: key.at,
                    value: key.value,
                    ease: key.ease.clone(),
                });
            }
        }
        for old in was {
            if !now.iter().any(|key| doc_view::near(old.at, key.at)) {
                commands.push(Command::ClearClipKey {
                    item,
                    property,
                    at: old.at,
                });
            }
        }
    }
    commands
}

/// The colour panel's rows: the adjust package's parameters, at the values
/// the clip's chain holds or at the defaults when the clip carries none.
/// Every one can be keyed; `at` is the playhead's place on the clip's
/// content time, or None when it is outside the clip - a keyed knob shows
/// its ride's value there, and its cluster says whether a key sits under
/// the playhead.
fn adjust_rows(chain: &[&Effect], at: Option<Tick>) -> Vec<AppliedParamData> {
    let Some(package) = Catalogue::builtin().get(ADJUST_ID) else {
        return Vec::new();
    };
    let held = chain
        .iter()
        .copied()
        .find(|entry| entry.package == ADJUST_ID);
    package
        .manifest
        .params
        .iter()
        // The wheels and curves are drawn by widgets of their own.
        .filter(|param| !param.kind.is_compound())
        .map(|param| {
            let keyed = held.is_some_and(|entry| doc_view::param_keyed(entry, &param.key));
            let (here, prev, next) = match (held, at) {
                (Some(entry), Some(at)) if keyed => {
                    let keys = doc_view::param_keys(entry, &param.key);
                    let (prev, next) = doc_view::keys_around(keys, at);
                    (
                        doc_view::key_near(keys, at).is_some(),
                        prev.is_some(),
                        next.is_some(),
                    )
                }
                _ => (false, false, false),
            };
            let value = match (held, at) {
                (Some(entry), Some(at)) => doc_view::param_at(entry, &param.key, at, param.default),
                (Some(entry), None) => doc_view::param_or(entry, &param.key, param.default),
                (None, _) => param.default,
            };
            AppliedParamData {
                entry: -1,
                key: param.key.as_str().into(),
                label: param.label.as_str().into(),
                group: param.group.as_str().into(),
                unit: param.unit.as_str().into(),
                min: param.min as f32,
                max: param.max as f32,
                step: if param.step > 0.0 {
                    param.step as f32
                } else {
                    ((param.max - param.min) / 200.0) as f32
                },
                default_value: param.default as f32,
                value: value as f32,
                fmt: format_of(&param.unit),
                keyable: true,
                keyed,
                here,
                prev,
                next,
            }
        })
        .collect()
}

/// A colour knob's value as the document holds it: RGBA packed into one
/// number, red in the top byte, as the shader unpacks it.
fn packed(colour: slint::Color) -> f64 {
    let bytes = [colour.red(), colour.green(), colour.blue(), colour.alpha()];
    f64::from(u32::from_be_bytes(bytes))
}

/// The colour a knob's packed number holds.
fn unpacked(value: f64) -> slint::Color {
    let [r, g, b, a] = (value.clamp(0.0, f64::from(u32::MAX)) as u32).to_be_bytes();
    slint::Color::from_argb_u8(a, r, g, b)
}

/// A chain's colour knobs, a row each, holding the document's colour or
/// the default. A knob whose default is no colour at all - an alpha of
/// zero - is one its package finds by itself, a key's screen, and says so
/// while it holds that. `pickable` offers the eyedropper, for a clip with a
/// picture of its own to take the colour off; `picking` is the knob, by
/// link and key, whose eyedropper is armed.
fn chain_colours(
    chain: &[&Effect],
    pickable: bool,
    picking: Option<(usize, &str)>,
) -> Vec<ColourKnobData> {
    let catalogue = Catalogue::builtin();
    chain
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let package = catalogue
                .packages()
                .find(|package| package.answers_to(&entry.package))?;
            Some((index, *entry, package))
        })
        .flat_map(|(index, entry, package)| {
            package
                .manifest
                .params
                .iter()
                .filter(|param| param.kind == ParamType::Color)
                .map(move |param| {
                    let value = doc_view::param_or(entry, &param.key, param.default);
                    let can_automatic = unpacked(param.default).alpha() == 0;
                    ColourKnobData {
                        entry: index as i32,
                        key: param.key.as_str().into(),
                        label: param.label.as_str().into(),
                        value: unpacked(value),
                        automatic: can_automatic && unpacked(value).alpha() == 0,
                        can_automatic,
                        pickable,
                        picking: picking == Some((index, param.key.as_str())),
                    }
                })
        })
        .collect()
}

/// A chain's choices - its enum knobs - a row of segments each, at the
/// document's value or the default.
fn chain_choices(chain: &[&Effect]) -> Vec<ChainChoiceData> {
    let catalogue = Catalogue::builtin();
    let mut choices = Vec::new();
    for (index, entry) in chain.iter().enumerate() {
        let Some(package) = catalogue
            .packages()
            .find(|package| package.answers_to(&entry.package))
        else {
            continue;
        };
        for param in &package.manifest.params {
            if param.kind != ParamType::Enum || param.values.is_empty() {
                continue;
            }
            let value = doc_view::param_or(entry, &param.key, param.default);
            let options: Vec<SharedString> = param
                .values
                .iter()
                .enumerate()
                .map(|(at, number)| {
                    param
                        .labels
                        .get(at)
                        .cloned()
                        .unwrap_or_else(|| format!("{number}"))
                        .into()
                })
                .collect();
            choices.push(ChainChoiceData {
                entry: index as i32,
                key: param.key.as_str().into(),
                label: param.label.as_str().into(),
                options: ModelRc::new(VecModel::from(options)),
                values: ModelRc::new(VecModel::from(
                    param
                        .values
                        .iter()
                        .map(|number| *number as f32)
                        .collect::<Vec<_>>(),
                )),
                current: param
                    .values
                    .iter()
                    .position(|number| (number - value).abs() < 1e-9)
                    .unwrap_or(0) as i32,
            });
        }
    }
    choices
}

/// A chain as the inspector's stack draws it: one row per link, and one per
/// knob its package declares, holding the document's value or the default.
/// A link no package answers to keeps its row - so it can be removed - and
/// gets no knobs.
fn chain_rows(chain: &[&Effect]) -> (Vec<AppliedEntryData>, Vec<AppliedParamData>) {
    let catalogue = Catalogue::builtin();
    let mut rows = Vec::new();
    let mut knobs = Vec::new();
    for (index, entry) in chain.iter().enumerate() {
        let package = catalogue
            .packages()
            .find(|package| package.answers_to(&entry.package));
        rows.push(AppliedEntryData {
            id: entry.package.as_str().into(),
            name: package
                .map(|package| package.manifest.effect.name.clone())
                .unwrap_or_else(|| label_of(&entry.package))
                .into(),
            on: entry.enabled,
            known: package.is_some(),
        });
        let Some(package) = package else { continue };
        // The adjust link shows as a link - it can be bypassed or removed
        // here - but its knobs are the Adjust tab's, not the chain's.
        if package.id() == ADJUST_ID {
            continue;
        }
        // Every filter has an intensity whether or not it says so: the one
        // slider a look is expected to have. First, above the look's own.
        if package.kind() == PackageKind::Filter {
            knobs.push(AppliedParamData {
                entry: index as i32,
                key: concat_effects::catalogue::INTENSITY.into(),
                label: t("studio.intensity").into(),
                group: "".into(),
                unit: "%".into(),
                min: 0.0,
                max: 100.0,
                step: 1.0,
                default_value: 100.0,
                value: doc_view::param_or(entry, concat_effects::catalogue::INTENSITY, 100.0)
                    as f32,
                fmt: ParamFormat::Percent,
                keyable: false,
                keyed: false,
                here: false,
                prev: false,
                next: false,
            });
        }
        // A colour is a swatch of its own (see `chain_colours`), a choice
        // a row of segments (see `chain_choices`), and a wheel or a curve is
        // drawn apart (see `chain_grading`).
        for param in package.manifest.params.iter().filter(|param| {
            !param.kind.is_compound()
                && param.kind != ParamType::Color
                && param.kind != ParamType::Enum
        }) {
            let step = if param.step > 0.0 {
                param.step
            } else {
                (param.max - param.min) / 200.0
            };
            knobs.push(AppliedParamData {
                entry: index as i32,
                key: param.key.as_str().into(),
                label: param.label.as_str().into(),
                group: "".into(),
                unit: param.unit.as_str().into(),
                min: param.min as f32,
                max: param.max as f32,
                step: step as f32,
                default_value: param.default as f32,
                value: doc_view::param_or(entry, &param.key, param.default) as f32,
                fmt: format_of(&param.unit),
                keyable: false,
                keyed: false,
                here: false,
                prev: false,
                next: false,
            });
        }
    }
    (rows, knobs)
}

/// A link's wheels and curves as the inspector draws them: the values the
/// link holds, or the defaults when there is no link yet. `entry` is its
/// index in the chain, -1 for the Adjust panel; `keys` is the playhead's
/// place on the clip's content time when the wheels can carry keys -
/// Some(None) when it is outside the clip - and None when they cannot.
fn grading_rows(
    package: &concat_effects::Package,
    link: Option<&Effect>,
    entry: i32,
    keys: Option<Option<Tick>>,
) -> (Vec<WheelData>, Vec<(CurveData, Vec<CurvePointData>)>) {
    // The knobs' own values: what a curve's points are read from.
    let params = link.map(doc_view::plain_params).unwrap_or_default();
    let params = &params;
    let mut wheels = Vec::new();
    let mut curves = Vec::new();
    for param in &package.manifest.params {
        match param.kind {
            ParamType::Wheel => {
                let [x, y, m] = grading::wheel_keys(&param.key);
                let at = keys.flatten();
                let value = |name: &str, fallback: f64| match (link, at) {
                    (Some(link), Some(at)) => doc_view::param_at(link, name, at, fallback),
                    _ => params.get(name).copied().unwrap_or(fallback),
                };
                let keyed = link.is_some_and(|link| doc_view::param_keyed(link, &m));
                let (here, prev, next) = match (link, at) {
                    (Some(link), Some(at)) if keyed => {
                        let held = doc_view::param_keys(link, &m);
                        let (prev, next) = doc_view::keys_around(held, at);
                        (
                            doc_view::key_near(held, at).is_some(),
                            prev.is_some(),
                            next.is_some(),
                        )
                    }
                    _ => (false, false, false),
                };
                wheels.push(WheelData {
                    entry,
                    key: param.key.as_str().into(),
                    label: param.label.as_str().into(),
                    group: param.group.as_str().into(),
                    x: value(&x, 0.0) as f32,
                    y: value(&y, 0.0) as f32,
                    master: value(&m, param.default) as f32,
                    min: param.min as f32,
                    max: param.max as f32,
                    default_value: param.default as f32,
                    keyable: keys.is_some(),
                    keyed,
                    here,
                    prev,
                    next,
                });
            }
            ParamType::Curve => {
                let points: Vec<CurvePointData> = grading::stored_points(params, &param.key)
                    .into_iter()
                    .map(|(index, x, y)| CurvePointData {
                        index: index as i32,
                        x: x as f32,
                        y: y as f32,
                    })
                    .collect();
                curves.push((
                    CurveData {
                        entry,
                        key: param.key.as_str().into(),
                        label: param.label.as_str().into(),
                        group: param.group.as_str().into(),
                        points: ModelRc::default(),
                        path: grading::curve_path(params, &param.key).into(),
                    },
                    points,
                ));
            }
            _ => {}
        }
    }
    (wheels, curves)
}

/// Every link's wheels and curves in a chain, for the stack.
fn chain_grading(chain: &[&Effect]) -> (Vec<WheelData>, Vec<(CurveData, Vec<CurvePointData>)>) {
    let catalogue = Catalogue::builtin();
    let mut wheels = Vec::new();
    let mut curves = Vec::new();
    for (index, link) in chain.iter().enumerate() {
        let Some(package) = catalogue
            .packages()
            .find(|package| package.answers_to(&link.package))
        else {
            continue;
        };
        if package.id() == ADJUST_ID {
            continue;
        }
        let (more_wheels, more_curves) = grading_rows(package, Some(*link), index as i32, None);
        wheels.extend(more_wheels);
        curves.extend(more_curves);
    }
    (wheels, curves)
}

/// The document keys an Adjust knob is keyed under, with what each is worth
/// with no key: a wheel's three - named by the wheel or by any of its own -
/// else the knob's own.
fn adjust_key_names(key: &str) -> Vec<(String, f64)> {
    let Some(package) = Catalogue::builtin().get(ADJUST_ID) else {
        return vec![(key.to_owned(), 0.0)];
    };
    match package.manifest.owner(key) {
        Some(param) if param.kind == ParamType::Wheel => {
            let [x, y, m] = grading::wheel_keys(&param.key);
            vec![(x, 0.0), (y, 0.0), (m, param.default)]
        }
        Some(param) => vec![(key.to_owned(), param.default)],
        None => vec![(key.to_owned(), 0.0)],
    }
}

/// The keys that move with an Adjust knob's key in the keyframes pane: for
/// a wheel's master, the puck's two; nothing for any other knob.
fn wheel_partners(param: &str) -> Vec<String> {
    let names = adjust_key_names(param);
    if names.len() == 3 && names[2].0 == param {
        names.into_iter().take(2).map(|(name, _)| name).collect()
    } else {
        Vec::new()
    }
}

/// The commands that take a wheel's puck keys at `from` to `to` with its
/// master's key - their values and eases kept - when `param` is a wheel's
/// master; nothing for any other knob.
fn partners_moved(link: &Effect, item: ItemId, param: &str, from: Tick, to: Tick) -> Vec<Command> {
    wheel_partners(param)
        .into_iter()
        .filter_map(|name| {
            let key = doc_view::key_near(doc_view::param_keys(link, &name), from)?.clone();
            Some([
                Command::ClearEffectKey {
                    item,
                    effect: link.id,
                    param: name.clone(),
                    at: key.at,
                },
                Command::SetEffectKey {
                    item,
                    effect: link.id,
                    param: name,
                    at: to,
                    value: key.value,
                    ease: key.ease,
                },
            ])
        })
        .flatten()
        .collect()
}

/// The effects a project's clips name that no package answers to - retired
/// ones, or packages not installed here - by name, each once, in the order
/// they first appear.
fn retired_effects(doc: &Doc) -> Vec<String> {
    let catalogue = Catalogue::builtin();
    let mut names: Vec<String> = Vec::new();
    for (_, item) in doc.items.iter() {
        for link in doc_view::chain(&item.effects) {
            if catalogue
                .packages()
                .any(|package| package.answers_to(&link.package))
            {
                continue;
            }
            let name = label_of(&link.package);
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// What the confirmation sheet can ask about.
#[derive(Clone, Debug, PartialEq)]
pub enum Confirm {
    /// Deleting a whole lane, and every clip on it.
    RemoveTrack { lane: LaneId },
    /// Exporting over a file that is already at `path`.
    ReplaceExport { path: String },
    /// Quitting though the project could not be saved, for `error`. A
    /// close that cannot write the project keeps the window, and without
    /// this a disk unplugged, full or read-only made a window that would
    /// never close.
    QuitWithoutSaving { error: String },
    /// Restarting for a setting that is read only at launch: the project
    /// is saved, and opens again in the new window.
    Restart,
    /// Applying a new address for the server, which restarts it and so
    /// stops the `jobs` scripts and apps have running.
    StopServerJobs { jobs: usize },
}

/// One clip on the clipboard: the clip as it was when copied, the place
/// in its timeline's stack of the lane it sat on, counted from the bottom,
/// and the file it plays. A lane's id belongs to its timeline, so a paste
/// into another timeline goes by that place instead and lands the clip on
/// the same lane there (#290). A media id belongs to its project, so a
/// paste into another project goes by the file: the same id there is
/// another file or none.
#[derive(Clone, Debug)]
pub struct Held {
    pub row: usize,
    pub clip: Item,
    pub media: Option<model::Media>,
}

/// Where each held clip lands: `(row, start)` per clip in the order held,
/// or `None` for one with no lane to land on. The group keeps its shape,
/// every clip the same distance from the earliest as when copied, from
/// `base` on; its rows are shifted by `shift` and held inside the `lanes`
/// of the timeline it lands in, and a locked row is passed over for the
/// nearest open one.
fn paste_plan(
    held: &[Held],
    base: Tick,
    shift: isize,
    lanes: usize,
    locked: &dyn Fn(usize) -> bool,
) -> Vec<Option<(usize, Tick)>> {
    let origin = held
        .iter()
        .map(|held| held.clip.start)
        .min()
        .unwrap_or(Tick::ZERO);
    let open: Vec<usize> = (0..lanes).filter(|row| !locked(*row)).collect();
    held.iter()
        .map(|held| {
            let top = isize::try_from(lanes.checked_sub(1)?).ok()?;
            let wanted = (isize::try_from(held.row).ok()? + shift).clamp(0, top);
            let wanted = usize::try_from(wanted).ok()?;
            let row = if locked(wanted) {
                *open.iter().min_by_key(|row| row.abs_diff(wanted))?
            } else {
                wanted
            };
            Some((row, base + (held.clip.start - origin)))
        })
        .collect()
}

/// Each rewritten link as "Old → New", the new one by its package's name.
fn upgraded_effects(pairs: &[(String, String)]) -> Vec<String> {
    let catalogue = Catalogue::builtin();
    pairs
        .iter()
        .map(|(old, new)| {
            let new = catalogue
                .get(new)
                .map(|package| package.manifest.effect.name.clone())
                .unwrap_or_else(|| label_of(new));
            format!("{} → {new}", label_of(old))
        })
        .collect()
}

fn label_of(id: &str) -> String {
    // The name after the namespace: `concat.camera-shake` is Camera Shake.
    let id = id.rsplit('.').next().unwrap_or(id);
    id.split(['-', '_'])
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Studio {
    /// A window with nothing open: the launch screen, with the recents list
    /// read off disk.
    pub fn new(host: Host) -> Self {
        let prefs = Preferences::load(&host.dirs);
        // The panels where the last session left them, while Settings says
        // to; a saved dock this build cannot read is the default instead.
        let saved = prefs.layout.as_ref().filter(|_| prefs.remembers_layout());
        let dock = saved
            .and_then(|layout| layout.wide.as_ref())
            .and_then(Dock::from_saved)
            .unwrap_or_else(default_dock);
        let dock_aside = saved
            .and_then(|layout| layout.compact.as_ref())
            .and_then(Dock::from_saved)
            .unwrap_or_else(crate::dock::compact_dock);
        let mut start = crate::panes::start::StartPane::default();
        if let Some(location) = prefs.project_location() {
            start.location = location.to_owned();
        }
        // The remembered log level, unless this run's environment named
        // one: CONCAT_LOG is the louder ask.
        if concat_host::logs::environment_level().is_none()
            && let Some(level) = prefs
                .log_level
                .as_deref()
                .and_then(concat_host::logs::level_named)
            && level != concat_host::logs::level()
        {
            concat_host::logs::set_level(level);
        }
        // The words first, so everything published from here on is in
        // the remembered language.
        i18n::select(prefs.locale.as_deref().unwrap_or(i18n::ENGLISH), &host.dirs);
        let languages = i18n::languages(&host.dirs);
        let recents = projects::recents(&host.dirs.config);
        let text_presets = presets::all(&host.dirs);
        let mut studio = Self {
            prefs,
            library: Default::default(),
            session: None,
            echo: None,
            document: Document::default(),
            dirty: false,
            autosave: slint::Timer::default(),
            media: crate::panes::media_bin::MediaBin::default(),
            peaks: HashMap::new(),
            beats: HashMap::new(),
            beats_pending: HashSet::new(),
            beats_failed: HashSet::new(),
            strips: HashMap::new(),
            windows: HashMap::new(),
            art_pending: HashSet::new(),
            art_failed: HashSet::new(),
            art_stamp: None,
            proxied: HashSet::new(),
            window_pending: HashSet::new(),
            waves: RefCell::new(HashMap::new()),
            lanes: crate::panes::timeline::TimelinePane::default(),
            selection: Vec::new(),
            media_focused: false,
            playhead: 0.0,
            // ~20px a second: a ten-second cut fits a pane at its default width.
            playing: false,
            transport: slint::Timer::default(),
            coast: crate::panes::glide::Coast::default(),
            glide_beat: slint::Timer::default(),
            hover: None,
            hover_hush: slint::Timer::default(),
            clipboard: Vec::new(),
            monitor: crate::panes::monitor::MonitorPane::default(),
            export: Default::default(),
            settings: crate::panes::settings::SettingsPane::default(),
            relink: crate::panes::relink::RelinkPane::default(),
            unread_pending: None,
            confirm: None,
            transition_selected: None,
            preview_full: false,
            full_screen_shown: false,
            window_was_full: false,
            open_menu: -1,
            menu_bar_token: 0,
            menu_target: None,
            menu_media: None,
            menu_token: 0,
            activity_burst: None,
            toast: ToastState::default(),
            on_start: true,
            start,
            threads: crate::panes::threads::ThreadsPane::default(),
            recents,
            posters: HashMap::new(),
            posters_pending: HashSet::new(),
            lengths: HashMap::new(),
            project_name: t("common.untitledProject"),
            dock,
            dock_aside,
            compact: false,
            speech_docked: false,
            workspace: (0.0, 0.0),
            divider_press: None,
            gesture: Gesture::None,
            stage_guides: Vec::new(),
            inspector_jump: (0, "", ""),
            media_jump: 0,
            captions_jump: 0,
            audition: None,
            revision: 0,
            flat: None,
            titles_painting: false,
            titles_tried: None,
            commit_pending: false,
            commit_targets: Vec::new(),
            words: None,
            commit_timer: slint::Timer::default(),
            layout_timer: slint::Timer::default(),
            shelf_stamp: std::cell::RefCell::new(None),
            look_art: std::cell::RefCell::new(HashMap::new()),
            card_paths: HashMap::new(),
            cards_drawn: 0,
            cards_busy: false,
            cards_again: false,
            packages_watch: slint::Timer::default(),
            packages_seen: 0,
            packages_pending: None,
            last_commit: None,
            title_blocks: HashMap::new(),
            drop: None,
            project_sheet: crate::panes::project::ProjectPane::default(),
            captions: crate::panes::captions::CaptionsPane::default(),
            speech: crate::panes::speech::SpeechPane::with_saved(&host.dirs),
            sounds: crate::panes::sounds::SoundsPane::open(&host.dirs),
            source: None,
            meters: crate::meters::MeterFeed::default(),
            voiceover: crate::panes::voiceover::VoiceoverPane::default(),
            text_presets,
            installed_fonts: presets::installed_fonts(&host.dirs),
            system_fonts: Vec::new(),
            fonts_published: std::cell::Cell::new(None),
            font_query: String::new(),
            font_search_published: std::cell::Cell::new(None),
            languages,
            brush: 0,
            brush_size: 0.06,
            painting: true,
            picking: None,
            cutout_jobs: HashMap::new(),
            stabilize_jobs: HashMap::new(),
            voice_jobs: HashMap::new(),
            voice_cancel: None,
            voice_failed: HashSet::new(),
            curve_selected: (String::new(), Vec::new()),
            speed_grabbed: None,
            stabilize_retry: HashSet::new(),
            failed_analyses: HashSet::new(),
            mask_focus: None,
            enhance_jobs: HashMap::new(),
            enhance_scale: 0.0,
            enhance_strength: concat_host::enhance::DEFAULT_STRENGTH,
            enhance_preview: None,
            enhance_previewing: None,
            enhance_ticked: HashSet::new(),
            reverse_jobs: HashMap::new(),
            merge_job: None,
            region_job: None,
            pending_stroke: None,
            tripped_strokes: HashSet::new(),
            host,
        };
        studio.handle(crate::panes::Msg::Settings(
            crate::panes::settings::SettingsMsg::Restore,
        ));
        // The effects shelf starts on "All", not whatever a stale default
        // would leave it on.
        studio.library[1].category = "All".to_owned();
        studio.library[1].group = -1;
        studio
    }
}

/// How a detach goes, or why it cannot; see [`detach_plan`].
#[derive(Debug, PartialEq, Eq)]
enum DetachPlan {
    /// The sound comes off.
    Detach,
    /// The picture is still muted by a detach whose sound clip has since
    /// been deleted: it is unmuted, then its sound comes off.
    Again,
    /// Not a video, or its file has no sound.
    NoSound,
    /// Its sound is on the timeline already, detached.
    Already,
}

/// How a detach of a clip goes, given what it is, whether its own sound
/// is silenced, whether its file has sound and whether a sound detached
/// from it is on the timeline. `DetachAudio` itself changes nothing when
/// it cannot happen, and says nothing; the last two answers are what the
/// window says instead.
fn detach_plan(kind: Kind, muted: bool, has_audio: bool, detached: bool) -> DetachPlan {
    if kind != Kind::Video || !has_audio {
        DetachPlan::NoSound
    } else if detached {
        DetachPlan::Already
    } else if muted {
        DetachPlan::Again
    } else {
        DetachPlan::Detach
    }
}

/// Whether a move puts any clip somewhere other than where `before` has
/// it: another lane, or another start. A move that names no lane keeps
/// the clip's own, and one of a clip that has gone is no move.
fn moved_any<'a>(moves: &[ClipMove], before: impl Fn(ItemId) -> Option<&'a Item>) -> bool {
    moves.iter().any(|wanted| {
        before(wanted.item).is_some_and(|clip| {
            wanted.lane.is_some_and(|lane| lane != clip.lane) || clip.start != wanted.start
        })
    })
}

/// The clips a flattened edit makes audible, as playback takes them; see
/// `concat_host::playback::audible`.
pub(crate) fn audio_specs(clips: &[concat_export::ExportClip]) -> Vec<ClipSpec> {
    concat_host::playback::audible(clips)
}

/// The edit point beside `playhead` among the `(start, duration)` spans,
/// the origin and the last span's end: the nearest after it when
/// `forward`, before it otherwise, and `playhead` itself when there is
/// none that way. Counted in whole frames at `rate`, so a playhead parked
/// on a cut is on it and the next press goes on to the one beyond,
/// however the seconds round.
fn edit_point(spans: &[(f64, f64)], playhead: f32, rate: f32, forward: bool) -> f32 {
    let fps = f64::from(rate.round().max(1.0));
    let frame = |seconds: f64| (seconds * fps).round() as i64;
    let here = frame(f64::from(playhead));
    let points = spans
        .iter()
        .flat_map(|&(start, duration)| [start, start + duration])
        .chain([0.0])
        .map(frame);
    let next = if forward {
        points.filter(|&point| point > here).min()
    } else {
        points.filter(|&point| point < here).max()
    };
    next.map_or(playhead, |point| (point as f64 / fps) as f32)
}

#[cfg(test)]
mod tests {
    use concat_doc::commands::{ClipMove, KeyProperty};
    use concat_doc::model::{Content, Ease, Item, ItemId, MediaKind, Motion, Move, VoiceKind};

    use super::{
        Command, Footprint, Studio, adjust_key_names, chain_choices, chain_colours, chain_rows,
        clip_edit_commands, colour_about, custom_frame, custom_rate, fps_of, grading_rows, has_hue,
        home_folder, key_commands, key_field_of, knob_range, packed, shown, wheel_partners,
        write_keyable,
    };
    use crate::doc_view::{self, Kind, bench::Bench, tick};

    /// A ten-second clip of a file of `kind` at the start of the main
    /// lane: the bench it is on, its id, and the clip.
    fn placed(kind: MediaKind) -> (Bench, ItemId, Item) {
        let mut bench = Bench::new();
        let media = bench.add_media("take", kind, 10.0);
        let lane = bench.main_lane();
        let id = bench.add_clip(media, lane, 0.0);
        let clip = doc_view::item(bench.doc(), id).expect("the clip").clone();
        (bench, id, clip)
    }

    /// The inspector writes its Voice isolation into the echo, and the
    /// commit is the difference between the echo and the document: a
    /// difference with no command would drop the choice, and the
    /// inspector's Voice only and No voice would spring back to Everything.
    #[test]
    fn a_voice_set_in_the_inspector_is_committed() {
        let (_, id, before) = placed(MediaKind::Audio);
        for mix in [
            doc_view::keep_voice(VoiceKind::Speech),
            doc_view::remove_voice(VoiceKind::Speech),
        ] {
            let mut after = before.clone();
            after.sound.voice = Some(mix.clone());
            let commands = clip_edit_commands(id, &before, &after, false);
            assert!(
                commands.iter().any(|command| matches!(
                    command,
                    Command::SetClipVoice { voice, .. } if voice.as_ref() == Some(&mix)
                )),
                "no command carries {mix:?}: {commands:?}"
            );
        }
        let mut on = before.clone();
        on.sound.voice = Some(doc_view::keep_voice(VoiceKind::Speech));
        assert!(
            clip_edit_commands(id, &on, &before, false)
                .iter()
                .any(|command| matches!(command, Command::SetClipVoice { voice: None, .. }))
        );
    }

    /// The Animate cards write the clip's motion into the echo; a commit
    /// with no command for motion would drop the pick, and the card would
    /// spring back to None. Picked and taken off, each lands.
    #[test]
    fn a_motion_picked_in_the_inspector_is_committed() {
        let (_, id, before) = placed(MediaKind::Video);
        let mut after = before.clone();
        after.picture.motion = Motion {
            enter: Some(Move {
                name: "pop".to_owned(),
                len: doc_view::len(0.5),
            }),
            ..Motion::default()
        };
        let patched = |commands: Vec<Command>| {
            commands.into_iter().find_map(|command| match command {
                Command::UpdateClip { patch, .. } => patch.motion,
                _ => None,
            })
        };
        assert_eq!(
            patched(clip_edit_commands(id, &before, &after, false)),
            Some(after.picture.motion.clone())
        );
        assert_eq!(
            patched(clip_edit_commands(id, &after, &before, false)),
            Some(Motion::default())
        );
    }
    /// A row rebuilt with fresh models of the same rows is the same row, so
    /// a publish leaves it - and the dropdown reading it - alone (#622);
    /// a changed option, or any other field, is a change.
    #[test]
    fn rows_holding_models_are_compared_by_what_the_models_hold() {
        use slint::{Model, ModelRc, SharedString, VecModel};
        let choice = |options: &[&str], current: i32| crate::ui::ChainChoiceData {
            options: ModelRc::new(VecModel::from(
                options
                    .iter()
                    .map(|o| SharedString::from(*o))
                    .collect::<Vec<_>>(),
            )),
            values: ModelRc::new(VecModel::from(vec![0.0f32, 1.0])),
            current,
            ..Default::default()
        };
        let shown = choice(&["Soft", "Hard"], 0);
        assert!(super::same_chain_choice(
            &shown,
            &choice(&["Soft", "Hard"], 0)
        ));
        assert!(!super::same_chain_choice(
            &shown,
            &choice(&["Soft", "Harder"], 0)
        ));
        assert!(!super::same_chain_choice(
            &shown,
            &choice(&["Soft", "Hard"], 1)
        ));

        let model = VecModel::from(vec![shown.clone()]);
        let held = model.row_data(0).expect("a row").options;
        super::sync_with(
            &model,
            vec![choice(&["Soft", "Hard"], 0)],
            super::same_chain_choice,
        );
        let after = model.row_data(0).expect("a row").options;
        assert!(held == after, "an unchanged row kept its model");
    }

    /// Up and Down walk the cuts on every lane in order, a playhead a
    /// hair off a cut by rounding counts as on it, and past the last cut
    /// (or before the first) the playhead stays where it is.
    #[test]
    fn the_edit_points_are_walked_in_order_across_lanes() {
        // One lane: 0-2, 2-5. Another: 1-3.
        let spans = [(0.0, 2.0), (2.0, 3.0), (1.0, 2.0)];
        let walk = |from: f32, forward: bool| super::edit_point(&spans, from, 30.0, forward);
        assert_eq!(walk(0.0, true), 1.0);
        assert_eq!(walk(1.0, true), 2.0);
        assert_eq!(walk(2.0, true), 3.0);
        assert_eq!(walk(3.0, true), 5.0);
        assert_eq!(walk(5.0, true), 5.0);
        assert_eq!(walk(5.0, false), 3.0);
        assert_eq!(walk(0.5, false), 0.0);
        assert_eq!(walk(0.0, false), 0.0);
        // A thousandth of a second past the cut at 2 is that cut's frame.
        assert_eq!(walk(2.001, true), 3.0);
        assert_eq!(walk(2.001, false), 1.0);
    }

    /// The three treatments are packages the Treatments switches turn on,
    /// so they must exist as sound packages; and they are switches, not
    /// cards, so the Library's sound shelf leaves them out while the rest
    /// of the sound library stays.
    #[test]
    fn the_treatments_are_switches_not_library_cards() {
        use concat_effects::catalogue::Catalogue;
        let audio: Vec<&str> = Catalogue::builtin()
            .of_kind(super::SHELF_KINDS[2])
            .map(|package| package.manifest.effect.id.as_str())
            .collect();
        for id in super::TREATMENTS {
            assert!(audio.contains(&id), "{id} is a sound package");
        }
        let view = Default::default();
        let (_, shelf) = super::shelves(
            super::SHELF_KINDS[2],
            &view,
            &[],
            &std::cell::RefCell::new(std::collections::HashMap::new()),
            &std::collections::HashMap::new(),
            |package| !super::TREATMENTS.contains(&package.manifest.effect.id.as_str()),
        );
        let ids: Vec<String> = shelf.iter().map(|entry| entry.id.to_string()).collect();
        for id in super::TREATMENTS {
            assert!(!ids.iter().any(|shown| shown == id), "{id} is not a card");
        }
        assert_eq!(ids.len(), audio.len() - super::TREATMENTS.len());
    }

    /// A text box's caret counts bytes and a run counts characters: an
    /// accented letter is one character of two bytes, an offset inside a
    /// character counts it, and one out of the text is held to it.
    #[test]
    fn a_caret_byte_is_the_character_it_falls_at() {
        let text = "été ok";
        assert_eq!(super::char_at(text, 0), 0);
        assert_eq!(super::char_at(text, 2), 1, "after é");
        assert_eq!(super::char_at(text, 1), 1, "inside é");
        assert_eq!(super::char_at(text, text.len() as i32), 6);
        assert_eq!(super::char_at(text, 1000), 6);
        assert_eq!(super::char_at(text, -1), 0);
    }

    /// Every built-in picture package ships a card, decodable at the size
    /// cards are drawn: one added without running scripts/bundle-cards.sh
    /// would show a sparkle on every machine that cannot draw its own.
    #[test]
    fn every_built_in_picture_package_ships_a_card() {
        for package in concat_effects::Catalogue::builtin().packages() {
            if !package.kind().is_visual() || package.id() == super::ADJUST_ID {
                continue;
            }
            let card = super::bundled_card(package.id())
                .unwrap_or_else(|| panic!("{} has no bundled card", package.id()));
            let size = card.size();
            assert_eq!(
                (size.width, size.height),
                (concat_host::cards::WIDTH, concat_host::cards::HEIGHT),
                "{}",
                package.id()
            );
        }
    }

    /// A pasted group keeps its shape from `base` on, its rows shifted
    /// and held inside the lanes it lands in, a locked row passed over
    /// for the nearest open one; with no lane at all nothing lands.
    #[test]
    fn a_paste_keeps_the_groups_shape_and_lands_on_rows_that_exist() {
        let lane = Bench::new().main_lane();
        let held = |row: usize, start: f64| super::Held {
            row,
            clip: Item::new(lane, tick(start), doc_view::len(1.0), Content::Layer),
            media: None,
        };
        let group = [held(1, 4.0), held(2, 5.5), held(0, 9.0)];
        let none = |_: usize| false;
        let at = |row: usize, start: f64| Some((row, tick(start)));
        // At the playhead on the rows copied from.
        let plan = super::paste_plan(&group, tick(10.0), 0, 4, &none);
        assert_eq!(plan, vec![at(1, 10.0), at(2, 11.5), at(0, 15.0)]);
        // After an anchor on row 3: the earliest lands there, the rest
        // keep their distance in rows, held inside the four lanes.
        let plan = super::paste_plan(&group, tick(20.0), 2, 4, &none);
        assert_eq!(plan, vec![at(3, 20.0), at(3, 21.5), at(2, 25.0)]);
        // Another timeline with two lanes: rows past the end fold back.
        let plan = super::paste_plan(&group, tick(0.0), 0, 2, &none);
        assert_eq!(plan, vec![at(1, 0.0), at(1, 1.5), at(0, 5.0)]);
        // A locked row is passed over for the nearest open one.
        let plan = super::paste_plan(&group, tick(0.0), 0, 4, &|row| row == 1);
        assert_eq!(plan, vec![at(0, 0.0), at(2, 1.5), at(0, 5.0)]);
        // No lanes, nothing lands; every lane locked, nothing lands.
        assert!(
            super::paste_plan(&group, tick(0.0), 0, 0, &none)
                .iter()
                .all(Option::is_none)
        );
        assert!(
            super::paste_plan(&group, tick(0.0), 0, 2, &|_| true)
                .iter()
                .all(Option::is_none)
        );
    }

    /// A detach that cannot happen has a reason to give, and a picture
    /// left muted by a deleted sound clip detaches again.
    #[test]
    fn a_detach_says_why_it_cannot_happen() {
        use super::{DetachPlan, detach_plan};
        assert_eq!(
            detach_plan(Kind::Video, false, true, false),
            DetachPlan::Detach
        );
        assert_eq!(
            detach_plan(Kind::Video, false, false, false),
            DetachPlan::NoSound
        );
        assert_eq!(
            detach_plan(Kind::Video, false, true, true),
            DetachPlan::Already
        );
        assert_eq!(
            detach_plan(Kind::Video, true, true, false),
            DetachPlan::Again
        );
        assert_eq!(
            detach_plan(Kind::Video, true, true, true),
            DetachPlan::Already
        );
        assert_eq!(
            detach_plan(Kind::Audio, false, true, false),
            DetachPlan::NoSound
        );
    }

    /// A scrubbed playhead lands on the nearest target inside the pull, and
    /// where it was asked to go when none is that near.
    #[test]
    fn a_scrub_is_pulled_to_the_nearest_edge_in_reach() {
        // Every number here is exact in binary, the edge of the pull too.
        let edges = [0.0, 2.0, 2.5, 5.0];
        assert_eq!(super::edit::pulled(2.125, 0.25, edges), 2.0);
        assert_eq!(super::edit::pulled(2.375, 0.25, edges), 2.5);
        assert_eq!(super::edit::pulled(3.5, 0.25, edges), 3.5);
        // On the edge of the pull is outside it.
        assert_eq!(super::edit::pulled(4.75, 0.25, edges), 4.75);
        assert_eq!(super::edit::pulled(1.0, 0.25, []), 1.0);
    }

    /// A click on a clip is a move to where it already is, and is no edit:
    /// its own lane, named or not, at its own start.
    #[test]
    fn a_click_moves_nothing() {
        let mut bench = Bench::new();
        let media = bench.add_media("take", MediaKind::Video, 2.0);
        let (lane, other) = (bench.main_lane(), bench.lane(0));
        let id = bench.add_clip(media, lane, 1.1);
        let clip = doc_view::item(bench.doc(), id).expect("the clip").clone();
        let before = |asked: ItemId| (asked == id).then_some(&clip);
        let to = |start: f64, lane| {
            vec![ClipMove {
                item: id,
                start: tick(start),
                lane,
            }]
        };
        assert!(!super::moved_any(&to(1.1, Some(lane)), before));
        assert!(!super::moved_any(&to(1.1, None), before));
        assert!(super::moved_any(&to(1.2, Some(lane)), before));
        assert!(super::moved_any(&to(1.1, Some(other)), before));
        // A clip that has gone is not a move.
        let gone = vec![ClipMove {
            item: concat_doc::commands::placeholder(),
            start: tick(9.0),
            lane: Some(lane),
        }];
        assert!(!super::moved_any(&gone, before));
    }
    /// A wheel is keyed as one knob: named by itself or by any of its own
    /// keys, it keys its puck and master; its master's key takes the puck's
    /// with it in the keyframes pane; any other knob keys alone.
    #[test]
    fn a_wheel_keys_as_one_knob() {
        let names = |key: &str| -> Vec<String> {
            adjust_key_names(key)
                .into_iter()
                .map(|(name, _)| name)
                .collect()
        };
        assert_eq!(names("lift"), ["lift.x", "lift.y", "lift.m"]);
        assert_eq!(names("gain.x"), ["gain.x", "gain.y", "gain.m"]);
        assert_eq!(names("exposure"), ["exposure"]);
        assert_eq!(wheel_partners("gamma.m"), ["gamma.x", "gamma.y"]);
        assert!(wheel_partners("gamma.x").is_empty());
        assert!(wheel_partners("exposure").is_empty());
    }

    /// The Adjust panel's wheels and curves come from its manifest: three
    /// wheels at rest and four straight curves on a clip that carries
    /// nothing, and a link's own values where it holds them.
    #[test]
    fn the_adjust_panel_draws_its_wheels_and_curves() {
        let package = concat_effects::Catalogue::builtin()
            .get("concat.adjust")
            .expect("a built-in");
        let (wheels, curves) = grading_rows(package, None, -1, Some(None));
        assert_eq!(
            wheels.iter().map(|w| w.key.as_str()).collect::<Vec<_>>(),
            ["lift", "gamma", "gain"]
        );
        assert!(
            wheels
                .iter()
                .all(|w| w.x == 0.0 && w.y == 0.0 && w.master == 0.0 && w.keyable)
        );
        assert_eq!(
            curves
                .iter()
                .map(|(c, _)| c.key.as_str())
                .collect::<Vec<_>>(),
            ["luma", "red", "green", "blue"]
        );
        assert!(curves.iter().all(|(_, points)| points.is_empty()));
        let mut link = doc_view::new_effect("concat.adjust");
        doc_view::set_param(&mut link, "gain.x", 0.25);
        doc_view::set_param(&mut link, "gain.m", -0.5);
        let mut values = doc_view::plain_params(&link);
        crate::grading::set_curve_point(&mut values, "red", -1, 0.5, 0.6);
        doc_view::set_plain_params(&mut link, &values);
        let (wheels, curves) = grading_rows(package, Some(&link), -1, None);
        assert_eq!((wheels[2].x, wheels[2].master), (0.25, -0.5));
        assert!(!wheels[2].keyable, "a stack's wheels take no keys");
        assert_eq!(curves[1].1.len(), 3, "the ends and the point put down");
    }

    /// A colour knob is a swatch in the stack, not a slider: at the chroma
    /// key's default it is the screen found by itself, and a colour picked
    /// is packed into the knob's number as the shader unpacks it, and read
    /// back the same, no longer found. The eyedropper is offered where the
    /// caller says, lit for the one knob armed.
    #[test]
    fn a_colour_knob_is_a_swatch() {
        let mut link = doc_view::new_effect("concat.chroma-key");
        let (_, knobs) = chain_rows(&[&link]);
        assert!(knobs.iter().all(|knob| knob.key != "color"), "no slider");
        assert!(
            knobs.iter().all(|knob| knob.key != "view"),
            "a choice, not a slider"
        );
        assert!(knobs.iter().any(|knob| knob.key == "strength"));
        let colours = chain_colours(&[&link], true, None);
        assert_eq!(colours.len(), 1);
        assert_eq!(colours[0].key, "color");
        assert!(colours[0].automatic && colours[0].can_automatic);
        assert!(colours[0].pickable && !colours[0].picking);
        let blue = slint::Color::from_argb_u8(255, 0x00, 0x47, 0xbb);
        doc_view::set_param(&mut link, "color", packed(blue));
        assert_eq!(
            doc_view::param(&link, "color"),
            Some(f64::from(0x0047_bbffu32))
        );
        let picked = chain_colours(&[&link], false, Some((0, "color")));
        assert_eq!(picked[0].value, blue);
        assert!(!picked[0].automatic && picked[0].can_automatic);
        assert!(!picked[0].pickable && picked[0].picking);
        let choices = chain_choices(&[&link]);
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].key, "view");
        assert_eq!(choices[0].current, 0);
    }
    /// A pick is the mean of the five by five pixels about the point, in
    /// light, and nothing off the picture; a grey has no hue to key.
    #[test]
    fn a_picked_colour_is_the_mean_about_the_point() {
        let mut frame = concat_core::frame::Frame::black(10, 10);
        for (at, pixel) in frame.pixels_mut().chunks_exact_mut(4).enumerate() {
            let shade = if at % 2 == 0 { 0 } else { 255 };
            pixel.copy_from_slice(&[0, shade, 64, 255]);
        }
        let [r, g, b] = colour_about(&frame, 0.5, 0.5).expect("on the picture");
        assert_eq!(r, 0);
        // The odd columns are lit: three of the five about the middle, so
        // three fifths of the light, which the display encoding writes at
        // 0.6 ^ (1 / 2.4) of its range.
        assert_eq!(g, 206);
        assert_eq!(b, 64);
        assert!(colour_about(&frame, 1.2, 0.5).is_none());
        assert!(has_hue([0, 177, 64]));
        assert!(!has_hue([128, 128, 128]));
        assert!(!has_hue([0, 0, 0]));
    }

    /// A typed frame is even on both sides and inside the limits.
    #[test]
    fn a_typed_frame_is_even_and_bounded() {
        assert_eq!(custom_frame(1920.0, 1080.0), (1920, 1080));
        assert_eq!(custom_frame(1081.0, 1079.0), (1082, 1080));
        assert_eq!(custom_frame(3.0, 99999.0), (16, 8192));
        assert_eq!(custom_frame(f32::NAN, -5.0), (16, 16));
    }

    /// A typed rate is the exact fraction: the NTSC ones as cameras record
    /// them, the rest to a thousandth in lowest terms, within the limits.
    #[test]
    fn a_typed_rate_is_an_exact_fraction() {
        assert_eq!(custom_rate(29.97), (30000, 1001));
        assert_eq!(custom_rate(23.976), (24000, 1001));
        assert_eq!(custom_rate(59.94), (60000, 1001));
        assert_eq!(custom_rate(30.0), (30, 1));
        assert_eq!(custom_rate(12.5), (25, 2));
        assert_eq!(custom_rate(0.2), (1, 1));
        assert_eq!(custom_rate(1000.0), (240, 1));
        assert!((fps_of(30000, 1001) - 29.97).abs() < 0.001);
    }

    const FRAME: (u32, u32) = (1920, 1080);

    /// A suggested folder sits under the home directory, with the
    /// platform's separators throughout, and is never empty on a machine
    /// that has a home: the launch sheet's Location starts from it.
    #[test]
    fn home_folder_is_under_home_with_native_separators() {
        let home = std::env::home_dir().expect("a home directory");
        let folder = std::path::PathBuf::from(home_folder("Desktop/Concat"));
        assert_eq!(folder, home.join("Desktop").join("Concat"));
        #[cfg(windows)]
        assert!(!folder.to_string_lossy().contains('/'));
    }

    /// A quarter turn swaps the bounds' pixel extents, which in fractions
    /// of a 16:9 frame is not a swap of the numbers.
    #[test]
    fn half_bounds_follow_the_turn() {
        let flat = Footprint {
            cx: 0.5,
            cy: 0.5,
            w: 0.5,
            h: 0.5,
            rotation: 0.0,
        };
        let (hw, hh) = flat.half_bounds(FRAME);
        assert!((hw - 0.25).abs() < 1e-9 && (hh - 0.25).abs() < 1e-9);
        let turned = Footprint {
            rotation: 90.0,
            ..flat
        };
        let (hw, hh) = turned.half_bounds(FRAME);
        // 540px tall becomes 540px wide: 270px each side of 1920.
        assert!((hw - 270.0 / 1920.0).abs() < 1e-9);
        assert!((hh - 480.0 / 1080.0).abs() < 1e-9);
    }

    /// The nearest pair inside the pull wins, and nothing outside it pulls.
    #[test]
    fn snap_picks_the_nearest_target_inside_the_pull() {
        // Centre 0.5 sits just off the frame's centre; the left edge at
        // 0.2 is nearer to another picture's edge at 0.21.
        let features = [0.2, 0.5, 0.8];
        let hit = Studio::stage_snap(features, &[0.0, 0.505, 1.0, 0.21], 0.02).unwrap();
        assert!((hit.0 - 0.005).abs() < 1e-9 && (hit.1 - 0.505).abs() < 1e-9);
        assert!(Studio::stage_snap(features, &[0.3, 0.6], 0.02).is_none());
    }

    /// A fitted 16:9 picture at rest covers the frame exactly.
    #[test]
    fn footprint_contains_an_unturned_box() {
        let box_ = Footprint {
            cx: 0.5,
            cy: 0.5,
            w: 0.5,
            h: 0.5,
            rotation: 0.0,
        };
        assert!(box_.contains(0.5, 0.5, FRAME));
        assert!(box_.contains(0.26, 0.26, FRAME));
        assert!(!box_.contains(0.24, 0.5, FRAME));
        assert!(!box_.contains(0.5, 0.76, FRAME));
    }

    /// Turned a quarter, a wide box stands tall: a point that was inside
    /// along the width is outside, and one above the old top edge is inside.
    /// The test is in pixels, which is where the turn happens — a fraction
    /// across is not the same distance as a fraction down.
    #[test]
    fn footprint_turns_in_pixels() {
        let box_ = Footprint {
            cx: 0.5,
            cy: 0.5,
            w: 0.5,
            h: 0.2,
            rotation: 90.0,
        };
        // Half the width is 480px, half the height 108px. After the turn
        // the box reaches 480px up and down and 108px either side.
        assert!(box_.contains(0.5, 0.5 + 400.0 / 1080.0, FRAME));
        assert!(!box_.contains(0.5, 0.5 + 500.0 / 1080.0, FRAME));
        assert!(box_.contains(0.5 + 100.0 / 1920.0, 0.5, FRAME));
        assert!(!box_.contains(0.5 + 120.0 / 1920.0, 0.5, FRAME));
    }

    /// The turn is clockwise, as the compositor's is. The unturned top-right
    /// corner sits at (480, -270) from the centre; turned thirty degrees
    /// clockwise in y-down pixels it lands at about (551, +6) - it has swung
    /// *down* past the centre line. A point just inside that corner is in
    /// the box, and its mirror above the line is not; turned the other way
    /// both answers would flip.
    #[test]
    fn footprint_turns_clockwise() {
        let box_ = Footprint {
            cx: 0.5,
            cy: 0.5,
            w: 0.5,
            h: 0.5,
            rotation: 30.0,
        };
        let x = 0.5 + 540.0 / 1920.0;
        assert!(box_.contains(x, 0.5 + 6.0 / 1080.0, FRAME));
        assert!(!box_.contains(x, 0.5 - 6.0 / 1080.0, FRAME));
    }

    /// A batch's position and turn move by the knob's change and its scale
    /// by the knob's factor; the first clip, the knob's reference, lands on
    /// the knob's value, and a field that is not a placement is set outright.
    #[test]
    fn a_batch_keeps_its_shape_about_the_first_clip() {
        use super::{ClipField, batch_value, relative_property};
        // The first clip was at 0.1 and the knob now reads 0.3: everyone
        // moves right by 0.2.
        assert_eq!(batch_value(ClipField::OffsetX, 0.3, 0.1, 0.1), 0.3);
        assert!((batch_value(ClipField::OffsetX, 0.3, 0.1, -0.5) - -0.3).abs() < 1e-12);
        assert_eq!(batch_value(ClipField::Rotation, 90.0, 0.0, 45.0), 135.0);
        // Scale is a factor: the first doubled, so the second doubles.
        assert_eq!(batch_value(ClipField::Scale, 2.0, 1.0, 0.5), 1.0);
        // A size is a size for every title in the batch.
        assert_eq!(batch_value(ClipField::FontSize, 0.08, 0.05, 0.02), 0.08);
        assert_eq!(relative_property(ClipField::FontSize), None);
        // Volume is absolute: different starting gains land on the same level.
        assert_eq!(relative_property(ClipField::Volume), None);
        for current in [0.0, 0.25, 1.0, 2.0] {
            assert_eq!(batch_value(ClipField::Volume, 0.5, 1.0, current), 0.5);
        }
    }

    /// A knob over a property with no keys writes its value; over one
    /// that rides, it writes the key at the playhead and leaves the value
    /// alone. The echo's keys then become one command each, and the
    /// playhead's place on a clip's content is held to the clip's ends.
    #[test]
    fn a_keyed_property_is_written_as_a_key_and_committed_as_commands() {
        let (_, id, before) = placed(MediaKind::Video);
        let mut after = before.clone();
        write_keyable(&mut after, KeyProperty::Scale, 2.0, tick(5.0));
        assert_eq!(after.picture.scale.value, 2.0, "no keys: the value");
        assert!(key_commands(id, &before, &after).is_empty());

        doc_view::set_key(&mut after.picture.scale.keys, tick(2.5), 1.0, Ease::LINEAR);
        write_keyable(&mut after, KeyProperty::Scale, 3.0, tick(7.5));
        assert_eq!(
            after.picture.scale.value, 2.0,
            "keyed: the value is left alone"
        );
        assert_eq!(shown(&after, KeyProperty::Scale, tick(7.5)), 3.0);
        let commands = key_commands(id, &before, &after);
        assert_eq!(commands.len(), 2, "one per key: {commands:?}");
        assert!(commands.iter().all(|command| matches!(
            command,
            Command::SetClipKey {
                property: KeyProperty::Scale,
                ..
            }
        )));

        let mut gone = after.clone();
        gone.picture
            .scale
            .keys
            .retain(|key| !doc_view::near(key.at, tick(2.5)));
        let commands = key_commands(id, &after, &gone);
        assert!(
            matches!(commands.as_slice(), [Command::ClearClipKey { at, .. }] if *at == tick(2.5)),
            "{commands:?}"
        );
        assert_eq!(doc_view::content_at(&after, tick(2.5)), tick(2.5));
        assert_eq!(
            doc_view::content_at(&after, tick(12.0)),
            tick(10.0),
            "held to the end"
        );
    }

    /// Width and height key the way scale does: a knob on an unkeyed pull
    /// sets its value, and once it carries keys the knob writes a key
    /// that the commit carries, while the other axis is left as it was.
    /// Every keyable property has a field that names it and back, which
    /// is what lets an inspector row and a Keyframes row find each other.
    #[test]
    fn a_pull_is_keyed_like_a_scale() {
        let (_, id, before) = placed(MediaKind::Video);
        let mut after = before.clone();
        write_keyable(&mut after, KeyProperty::StretchX, 1.5, tick(5.0));
        assert_eq!(after.picture.stretch_x.value, 1.5, "no keys: the value");

        doc_view::set_key(
            &mut after.picture.stretch_x.keys,
            tick(0.0),
            1.0,
            Ease::LINEAR,
        );
        write_keyable(&mut after, KeyProperty::StretchX, 2.0, tick(10.0));
        assert_eq!(
            after.picture.stretch_x.value, 1.5,
            "keyed: the value is left alone"
        );
        assert_eq!(shown(&after, KeyProperty::StretchX, tick(5.0)), 1.5);
        assert_eq!(shown(&after, KeyProperty::StretchY, tick(5.0)), 1.0);
        let commands = key_commands(id, &before, &after);
        assert_eq!(commands.len(), 2, "one per key: {commands:?}");
        assert!(commands.iter().all(|command| matches!(
            command,
            Command::SetClipKey {
                property: KeyProperty::StretchX,
                ..
            }
        )));

        for property in doc_view::KEY_PROPERTIES {
            assert_eq!(
                Studio::key_property_of(key_field_of(property)),
                Some(property)
            );
            let (low, high) = knob_range(property);
            assert!(low < high, "{property:?}");
        }
        assert_eq!(
            knob_range(KeyProperty::StretchY),
            (0.1, 4.0),
            "the inspector's"
        );
    }

    /// A batch level lands on every clip as the inspector commits it - one
    /// batch of each clip's commands - and one undo takes all of it back:
    /// the plain clip's level and the keyed clip's whole ride, with no key
    /// left behind at the playhead.
    #[test]
    fn a_batch_level_is_one_undo_step_for_every_clip() {
        use concat_doc::model::Model;
        use concat_doc::{Budget, Entry, History, Origin};
        let mut bench = Bench::new();
        let media = bench.add_media("a.wav", MediaKind::Audio, 10.0);
        let lane = bench.main_lane();
        let ids = [
            bench.add_clip(media, lane, 0.0),
            bench.add_clip(media, lane, 20.0),
        ];
        // The second rides: full, then faded out.
        for (at, value) in [(0.0, 1.0), (10.0, 0.0)] {
            bench.apply(&Command::SetClipKey {
                item: ids[1],
                property: KeyProperty::Volume,
                at: tick(at),
                value,
                ease: Ease::LINEAR,
            });
        }
        let document = bench.document.clone();

        let mut commands = Vec::new();
        for id in ids {
            let before = doc_view::item(&document, id).expect("clip");
            let mut after = before.clone();
            crate::batch_volume::set_level(&mut after, before, 0.5, tick(0.0));
            commands.extend(clip_edit_commands(id, before, &after, false));
        }
        let batch = Command::Batch { commands };
        let mut history: History<Model, Command, Vec<ItemId>> =
            History::new(document.clone(), Budget::default());
        let outcome =
            concat_doc::commands::run(history.current(), &batch, Origin(0)).expect("applies");
        history
            .record(
                outcome.commit.expect("the batch changes the clips"),
                Entry::new(vec![batch], Vec::new(), Vec::new()),
            )
            .expect("the step is kept");
        let clip = |history: &History<Model, Command, Vec<ItemId>>, id: ItemId| {
            doc_view::item(history.current(), id).expect("clip").clone()
        };
        assert_eq!(clip(&history, ids[0]).sound.volume.value, 0.5);
        let ride: Vec<f64> = clip(&history, ids[1])
            .sound
            .volume
            .keys
            .iter()
            .map(|key| key.value)
            .collect();
        assert_eq!(ride, vec![0.5, 0.0], "the ride moved, no key was added");

        assert!(history.undo().is_some());
        for id in ids {
            assert_eq!(
                clip(&history, id),
                *doc_view::item(&document, id).expect("clip"),
                "one undo puts {id} back"
            );
        }
        assert!(history.undo().is_none(), "it was one step");
    }
}

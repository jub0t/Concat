// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

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

use concat_effects::Catalogue;
use concat_effects::manifest::Kind as PackageKind;
use concat_host::playback::ClipSpec;
use concat_host::{
    AnalyseRequest, Cutouts, EnhanceRequest, ProjectInfo, RegionRequest, Session, media, projects,
    templates,
};
use concat_media::{DecodeOptions, Decoder, FrameSource, Pyramid, jpeg};
use concat_project::commands::{ClipMove, ClipPatch, TrackFlag, TrimEdge};
use concat_project::model::{
    self, AppliedFilter, Clip, Project, TextAlign, TextStyle, Timeline, Track, Transition,
};
use concat_project::{Command, why_not_merge};
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::dock::{
    Dock, DockLayout, SEAT_GAP, default_dock, lay_out, nearest_row, row_at, row_top,
};
use crate::format::{
    WAVE_BAR, WAVE_PITCH, colour_of, frames_timecode, hex_of, hex_rgba, hex_with_alpha,
    wave_columns, wave_path, when_phrase,
};
use crate::host::{
    CachedStrip, Host, MediaArt, WindowArt, cached_media_art, cached_window_art, image_at,
    image_of, media_art, on_ui_in_project, spawn, spawn_art, spawn_in_project, spawn_strip,
    strip_window, window_art, window_span, window_start,
};
use crate::i18n::{self, t, tf};
use crate::panes::settings::installed;
use crate::prefs::Preferences;
use crate::presets::{self, TextPreset};
use crate::ui::*;

/// The monitor's output sizes, matching the picker's rows.
pub const OUTPUTS: [(i32, i32); 6] = [
    (1920, 1080),
    (3840, 2160),
    (1080, 1920),
    (1080, 1080),
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
pub const EXPORT_RATES: [(i64, i64); 3] = [(24, 1), (30, 1), (60, 1)];
/// Narrower than this, in logical pixels, and the workspace shows the
/// compact dock: a phone, a tablet held upright, a desktop window squeezed
/// to a corner. Four panes across less than this is four slivers.
pub const COMPACT_WIDTH: f32 = 860.0;
/// Megabits per second at 1080p30 for each quality tier, for the size
/// estimate; and the CRF each tier renders at.
pub(crate) const EXPORT_TIERS: [f32; 3] = [16.0, 8.0, 4.0];
pub(crate) const EXPORT_CRF: [u8; 3] = [16, 20, 26];
pub(crate) const AUDIO_BPS: f32 = 192_000.0;

/// The frame shapes the launch screen offers, as the ratio behind each
/// label: width over height.
///
/// A shape and a size rather than the four fixed frames this was. Those
/// offered one upright frame at one size and no square at all, so a phone
/// cut at 4K was not a thing the form could ask for. Every pair of indices
/// resolves through [`frame_size`], which is the one place that knows what
/// a label means in pixels.
pub const ASPECTS: [(&str, u32, u32); 4] = [
    ("16:9", 16, 9),
    ("9:16", 9, 16),
    ("1:1", 1, 1),
    ("4:3", 4, 3),
];

/// The sizes, as the *short* edge in pixels.
///
/// Which is what the "p" in 1080p has always counted, and the only reading
/// that survives turning a frame upright: 1080p landscape is 1920 x 1080
/// and 1080p vertical is 1080 x 1920, the same number of lines either way.
/// Naming the long edge instead would make a vertical 1080p a 1080 x 1920
/// frame at one moment and a 608 x 1080 frame at another.
pub const SIZES: [(&str, u32); 3] = [("720p", 720), ("1080p", 1080), ("4K", 2160)];

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

/// The frame rates, as exact fractions. 29.97 is 30000/1001 and never
/// anything else - a decimal is for reading, and export must never be handed
/// one.
pub const START_RATES: [(&str, i64, i64); 5] = [
    ("24", 24, 1),
    ("25", 25, 1),
    ("29.97", 30000, 1001),
    ("30", 30, 1),
    ("60", 60, 1),
];

/// What one effect library is showing: the words typed into it, the shelf
/// picked, and whether the star is down.
///
/// Held here rather than in the panel because the filtering is done here:
/// the model a library renders arrives already narrowed, which is what lets
/// it be laid out as a grid - a tile's place is its index, and an index only
/// means a place when every entry in the model is one that shows.
#[derive(Clone, Debug, Default)]
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

/// Which library a shelf index names. The numbers are the panel's own -
/// `PresetStack.shelf`, and `Library.view`'s shelf argument everywhere else -
/// and the order the tabs sit in.
pub const SHELF_KINDS: [PackageKind; 4] = [
    PackageKind::Filter,
    PackageKind::Effect,
    PackageKind::Audio,
    PackageKind::Transition,
];

/// `name` under the home directory, as a path string; empty when there is
/// no home to speak of, and the form then asks for a folder outright.
pub(crate) fn home_folder(name: &str) -> String {
    std::env::var("HOME")
        .map(|home| format!("{home}/{name}"))
        .unwrap_or_default()
}

/// The bottom-right notice: one at a time. The token is what the panel
/// watches, bumped per notice so the same sentence said twice blinks twice.
#[derive(Default)]
pub struct ToastState {
    pub token: i32,
    pub message: String,
    pub failed: bool,
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
    pub start: f32,
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
    },
    Trim {
        clip: String,
        edge: Edge,
        start: f32,
        duration: f32,
        source_start: f32,
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
    pub catalogue_audio: Rc<VecModel<CatalogueEntryData>>,
    pub catalogue_transitions: Rc<VecModel<CatalogueEntryData>>,
    pub effect_groups: Rc<VecModel<SharedString>>,
    pub filter_groups: Rc<VecModel<SharedString>>,
    pub audio_groups: Rc<VecModel<SharedString>>,
    pub transition_groups: Rc<VecModel<SharedString>>,
    /// The selected clip's two chains and their knobs.
    pub applied_visual: Rc<VecModel<AppliedEntryData>>,
    pub applied_audio: Rc<VecModel<AppliedEntryData>>,
    pub visual_params: Rc<VecModel<AppliedParamData>>,
    pub audio_params: Rc<VecModel<AppliedParamData>>,
    /// The colour panel's knobs.
    pub adjust_params: Rc<VecModel<AppliedParamData>>,
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
    pub speech_models: Rc<VecModel<SharedString>>,
    pub speech_model_details: Rc<VecModel<SharedString>>,
    pub speakers: Rc<VecModel<SharedString>>,
    pub speaker_details: Rc<VecModel<SharedString>>,
    pub speech_samples: Rc<VecModel<SharedString>>,
    pub speech_sample_waves: Rc<VecModel<SharedString>>,
    pub speech_sample_details: Rc<VecModel<SharedString>>,
    pub transcribers: Rc<VecModel<ModelData>>,
    pub voices: Rc<VecModel<ModelData>>,
    pub seats: Rc<VecModel<SeatBox>>,
    pub dividers: Rc<VecModel<DockDivider>>,
    pub recents: Rc<VecModel<RecentProjectData>>,
    /// The Text page's presets, published once from the loaded list.
    pub text_presets: Rc<VecModel<TextPresetData>>,
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
            catalogue_audio: Rc::new(VecModel::default()),
            catalogue_transitions: Rc::new(VecModel::default()),
            effect_groups: Rc::new(VecModel::default()),
            filter_groups: Rc::new(VecModel::default()),
            audio_groups: Rc::new(VecModel::default()),
            transition_groups: Rc::new(VecModel::default()),
            applied_visual: Rc::new(VecModel::default()),
            applied_audio: Rc::new(VecModel::default()),
            visual_params: Rc::new(VecModel::default()),
            audio_params: Rc::new(VecModel::default()),
            adjust_params: Rc::new(VecModel::default()),
            key_rows: Rc::new(VecModel::default()),
            key_editor_rows: Rc::new(VecModel::default()),
            key_marks: Rc::new(VecModel::default()),
            library_views: Rc::new(VecModel::default()),
            menu: Rc::new(VecModel::default()),
            bar: Rc::new(VecModel::default()),
            caption_models: Rc::new(VecModel::default()),
            speech_models: Rc::new(VecModel::default()),
            speech_model_details: Rc::new(VecModel::default()),
            speakers: Rc::new(VecModel::default()),
            speaker_details: Rc::new(VecModel::default()),
            speech_samples: Rc::new(VecModel::default()),
            speech_sample_waves: Rc::new(VecModel::default()),
            speech_sample_details: Rc::new(VecModel::default()),
            transcribers: Rc::new(VecModel::default()),
            voices: Rc::new(VecModel::default()),
            seats: Rc::new(VecModel::default()),
            dividers: Rc::new(VecModel::default()),
            recents: Rc::new(VecModel::default()),
            text_presets: Rc::new(VecModel::default()),
        }
    }
}

/// Republish a list into a live model without resetting it: rows that did
/// not change are not written, and the length changes a row at a time.
pub fn sync<T: Clone + PartialEq + 'static>(model: &VecModel<T>, next: Vec<T>) {
    let mut next = next;
    let shared = model.row_count().min(next.len());
    let tail = next.split_off(shared);
    for (row, value) in next.into_iter().enumerate() {
        if model.row_data(row).as_ref() != Some(&value) {
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

/// The window's state. See the module docs for what is whose.
pub struct Studio {
    pub host: Host,
    pub prefs: Preferences,
    /// What each effect library is showing, indexed the way `SHELF_KINDS`
    /// is: 0 filters, 1 effects, 2 audio.
    pub library: [LibraryView; 4],

    // ── the edit ──
    pub session: Option<Session>,
    /// A clone of the project a gesture is mutating. `project()` reads it
    /// while it exists; a command replaces it.
    pub echo: Option<Project>,
    /// Stands in for a project when none is open, so every reader has
    /// something to read.
    empty: Project,
    /// Unsaved changes, and the timer that writes them.
    dirty: bool,
    autosave: slint::Timer,

    // ── the bin ──
    pub media: crate::panes::media_bin::MediaBin,
    /// Decoded art by media id, and the ids a worker is decoding for.
    pub peaks: HashMap<String, Arc<Pyramid>>,
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
    art_stamp: Option<(u64, bool, usize)>,
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
    pub playhead: f32,
    pub playing: bool,
    transport: slint::Timer,
    /// The preview axis: the instant under the pointer while it crosses the
    /// lanes, which the monitor shows instead of the playhead's. None when
    /// the pointer is elsewhere, or the axis is off, or the cut is playing.
    pub hover: Option<f32>,
    /// Stops the burst of sound a hover with audio plays.
    hover_hush: slint::Timer,
    /// One clip, held for Paste.
    pub clipboard: Option<Clip>,
    /// The monitor: its frame, and the requests for the next.
    pub monitor: crate::panes::monitor::MonitorPane,

    // ── the sheets and menus ──
    pub export: crate::panes::export::ExportPane,
    pub settings: crate::panes::settings::SettingsPane,
    pub relink: crate::panes::relink::RelinkPane,
    pub open_menu: i32,
    pub menu_bar_token: i32,
    pub menu_target: Option<String>,
    /// The media the bin's menu was opened on, when it was the bin's menu
    /// and not a clip's: the two share one set of rows and one token.
    pub menu_media: Option<String>,
    pub menu_token: i32,
    pub toast: ToastState,

    // ── the launch screen ──
    pub on_start: bool,
    pub start: crate::panes::start::StartPane,
    pub recents: Vec<ProjectInfo>,
    pub posters: HashMap<String, slint::Image>,
    posters_pending: HashSet<String>,
    pub project_name: String,

    // ── the workspace ──
    pub dock: Dock,
    /// The dock not showing: the compact one while the window is wide,
    /// the wide one while it is compact. Swapped with `dock` as the
    /// window crosses `COMPACT_WIDTH`, so each keeps its shape.
    pub dock_aside: Dock,
    /// Whether `dock` is the compact one.
    pub compact: bool,
    pub workspace: (f32, f32),
    pub divider_press: Option<(usize, f32, f32)>,
    pub gesture: Gesture,
    /// The snap lines of a stage move in flight; empty between moves.
    pub stage_guides: Vec<StageGuideData>,
    /// Where the inspector should go, and a count that changes every time
    /// something is applied from the library; see `Editor.inspector-jump-token`.
    pub inspector_jump: (i32, &'static str, &'static str),
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
    commit_target: Option<String>,
    commit_timer: slint::Timer,
    /// What the catalogue shelves were last built from; while nothing in
    /// it changes the shelves are not rebuilt.
    shelf_stamp: std::cell::RefCell<Option<ShelfStamp>>,
    /// The card stills of the user's own looks, by package id, loaded once
    /// from the package folder's `preview.png` and kept: the shelves are
    /// rebuilt on every publish and a picture read from disk each time
    /// would be the slowest thing in the window.
    look_art: std::cell::RefCell<HashMap<String, slint::Image>>,
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
    /// Every speaker the voice engine offers, in its own order.
    /// The looks the Text page offers; see `presets`.
    pub text_presets: Vec<TextPreset>,

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
    /// The mask analyses running, by media id, and how far each has got.
    cutout_jobs: HashMap<String, (bool, f32)>,
    /// Enhance at work, by clip id: whether the model is still
    /// downloading, and how far along. One at a time, like every long job.
    enhance_jobs: HashMap<String, (bool, f32)>,
    /// The smart stroke being read, by the same name, while one is.
    region_job: Option<String>,
    /// The last smart stroke as the stage drew it, kept on screen from the
    /// release until the model has read what was under it, so the paint
    /// does not vanish before its answer arrives.
    pending_stroke: Option<(String, f32, bool)>,
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

/// One audio track's row in the Audio panel's list: the name the file gave
/// it or its number, and its channel layout.
fn audio_track_label(track: &model::AudioTrack, position: usize) -> String {
    let name = if track.title.is_empty() {
        tf("Track {0}", &[&(position + 1)])
    } else {
        track.title.clone()
    };
    let layout = match track.channels {
        1 => t("mono"),
        2 => t("stereo"),
        channels => tf("{0} channels", &[&channels]),
    };
    format!("{name} · {layout}")
}

fn kind_of(clip: &Clip) -> ClipKind {
    match clip.kind {
        model::ClipKind::Video => ClipKind::Video,
        model::ClipKind::Audio => ClipKind::Audio,
        model::ClipKind::Image => ClipKind::Image,
        model::ClipKind::Text => ClipKind::Text,
        model::ClipKind::Layer => ClipKind::Filter,
    }
}

fn align_of(align: TextAlign) -> TextAlignment {
    match align {
        TextAlign::Left => TextAlignment::Left,
        TextAlign::Center => TextAlignment::Center,
        TextAlign::Right => TextAlignment::Right,
    }
}

/// A title as it is first placed: the embedded face, centred, white.
fn new_title_style() -> TextStyle {
    TextStyle {
        content: "New title".to_owned(),
        font_family: "Hanken Grotesk".to_owned(),
        font_weight: 600.0,
        ..TextStyle::default()
    }
}

/// A label for a catalogue id: "black-white" reads as "Black White".
/// One kind's shelves for the inspector: the shelf labels, in catalogue
/// order, and every package on them with the index of its shelf.
fn shelves(
    kind: PackageKind,
    view: &LibraryView,
    favourites: &[String],
    look_art: &std::cell::RefCell<HashMap<String, slint::Image>>,
) -> (Vec<SharedString>, Vec<CatalogueEntryData>) {
    let mut groups: Vec<String> = Vec::new();
    let mut entries = Vec::new();
    let query = view.query.trim().to_lowercase();
    for package in Catalogue::builtin().of_kind(kind) {
        let meta = &package.manifest.effect;
        // The colour panel's own package: every picture clip can carry it,
        // and it has a tab, not a shelf.
        if meta.id == ADJUST_ID {
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
        let name = t(&meta.name);
        let description = t(&meta.description);
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
        // A user's look brings its own still, read once from its folder;
        // a built-in's is compiled into the window and looked up by id there.
        let art = match &package.folder {
            Some(folder) => look_art
                .borrow_mut()
                .entry(meta.id.clone())
                .or_insert_with(|| {
                    // The author's own still, or the one rendered for them.
                    slint::Image::load_from_path(&folder.join("preview.png"))
                        .or_else(|_| slint::Image::load_from_path(&folder.join("preview.jpg")))
                        .unwrap_or_default()
                })
                .clone(),
            None => slint::Image::default(),
        };
        entries.push(CatalogueEntryData {
            id: meta.id.as_str().into(),
            name: name.into(),
            category: t(&category).into(),
            group: group as i32,
            description: description.into(),
            favourite: starred,
            art,
        });
    }
    (
        groups
            .into_iter()
            .map(|group| SharedString::from(t(&group)))
            .collect(),
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
                if patch.text.is_some() {
                    fields.push("text");
                }
                if patch.video_effects.is_some() {
                    fields.push("video_effects");
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
fn key_field_of(property: model::KeyProperty) -> ClipField {
    match property {
        model::KeyProperty::Scale => ClipField::Scale,
        model::KeyProperty::OffsetX => ClipField::OffsetX,
        model::KeyProperty::OffsetY => ClipField::OffsetY,
        model::KeyProperty::Rotation => ClipField::Rotation,
        model::KeyProperty::Opacity => ClipField::Opacity,
        model::KeyProperty::Volume => ClipField::Volume,
    }
}

/// What the catalogue shelves are a function of; see `Studio::shelf_stamp`.
#[derive(PartialEq)]
struct ShelfStamp {
    catalogue: usize,
    lang: String,
    views: Vec<(String, i32, bool, String)>,
    favourites: Vec<String>,
}

/// The built-in colour package's id; see `adjust_rows` and `Studio::adjust_set`.
const ADJUST_ID: &str = "concat.adjust";

/// The picture every look's card is rendered from.
const REFERENCE_STILL: &[u8] = include_bytes!("../ui/assets/effect-previews/sharpen.jpg");

/// Makes a package folder under `dir` from the table at `path`, and
/// returns the package's id. The id is `user.` and the file's name slugged;
/// a second import of the same name replaces the first.
fn import_cube(dir: &std::path::Path, path: &std::path::Path) -> Result<String, String> {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut slug: String = stem
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        slug = "look".to_owned();
    }
    let id = format!("user.{slug}");
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let lut = concat_effects::cube::parse(&text)?;
    let folder = dir.join(&id);
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let name = stem.trim().to_owned();
    let manifest = format!(
        "format = {}\n\n[effect]\nid = \"{id}\"\nname = {name:?}\nkind = \"filter\"\ncategory = \"Imported\"\n\
         description = \"A look imported from a .cube table.\"\n\n[lut]\nfile = \"look.cube\"\n\n\
         [ffmpeg]\nchain = \"lut3d=file={{lut}}\"\n\n[wgsl]\nentry = \"effect.wgsl\"\n",
        concat_effects::FORMAT
    );
    std::fs::write(folder.join("effect.toml"), manifest).map_err(|error| error.to_string())?;
    std::fs::write(
        folder.join("effect.wgsl"),
        "// The table, and nothing else; the host mixes it by intensity.\n\
         fn effect(uv: vec2<f32>) -> vec4<f32> {\n    let c = sample(uv);\n    return vec4<f32>(lut(c.rgb), c.a);\n}\n",
    )
    .map_err(|error| error.to_string())?;
    std::fs::write(folder.join("look.cube"), &text).map_err(|error| error.to_string())?;
    // The card still: the reference picture through the table, the same
    // arithmetic the GPU's sampler does. Read through Slint's decoder from
    // a copy on disk, since the bytes live in the binary.
    let reference = reference_still(dir)?;
    let image = slint::Image::load_from_path(&reference).map_err(|error| error.to_string())?;
    let Some(pixels) = image.to_rgba8() else {
        return Err("the reference picture would not decode".to_owned());
    };
    let (width, height) = (pixels.width(), pixels.height());
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for pixel in pixels.as_slice() {
        let rgb = lut.sample([
            f32::from(pixel.r) / 255.0,
            f32::from(pixel.g) / 255.0,
            f32::from(pixel.b) / 255.0,
        ]);
        for channel in rgb {
            out.push((channel.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
        out.push(255);
    }
    let file =
        std::fs::File::create(folder.join("preview.png")).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&out)
        .map_err(|error| error.to_string())?;
    Ok(id)
}

/// The reference picture every card is rendered from, as a file in the
/// effects folder: the bytes live in the binary, and both Slint's decoder
/// and FFmpeg's want a path.
fn reference_still(dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let reference = dir.join("reference.jpg");
    if !reference.is_file() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        std::fs::write(&reference, REFERENCE_STILL).map_err(|error| error.to_string())?;
    }
    Ok(reference)
}

/// The card still for a package of the user's own that brought none: the
/// reference picture through the package's own chain at its defaults,
/// written beside its manifest as `preview.jpg`, the way the built-ins'
/// cards were made. A package with a shader and no chain has no CPU
/// rendering to be had here, so its card stays blank until its author
/// puts a `preview.png` beside the manifest.
fn render_card(
    dir: &std::path::Path,
    package: &concat_effects::Package,
    chain: &str,
) -> Result<(), String> {
    let Some(folder) = package.folder.as_deref() else {
        return Ok(());
    };
    let reference = reference_still(dir)?;
    let mut decoder = Decoder::open(&reference, &DecodeOptions::default().scaled_to(320, 180))
        .map_err(|error| error.to_string())?;
    let frame = decoder
        .next_frame()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the reference picture holds no frame".to_owned())?;
    let treated = concat_media::treat(&frame, chain).map_err(|error| error.to_string())?;
    let bytes = jpeg(&treated, 3).map_err(|error| error.to_string())?;
    std::fs::write(folder.join("preview.jpg"), bytes).map_err(|error| error.to_string())
}

/// The ease a key put on at `at` inherits: that of whichever key it joins
/// behind, so laying a run of keys down does not alternate between shapes.
/// The first key on a parameter has nothing to inherit and gets the
/// straight line.
fn ease_before(keys: &[model::ParamKey], at: f64) -> model::KeyEase {
    keys.iter()
        .rev()
        .find(|key| key.at < at)
        .map_or(model::KeyEase::LINEAR, |key| key.ease)
}

/// The playhead's place in `clip`, `0..=1`, held to its ends: where a keyed
/// property is shown and written from. `Studio::key_point` is the same
/// without the clamp, for the diamonds, which must not land a key on an end
/// nobody aimed at.
fn place_in(clip: &Clip, playhead: f32) -> f64 {
    if clip.duration <= 0.0 {
        return 0.0;
    }
    ((f64::from(playhead) - clip.start) / clip.duration).clamp(0.0, 1.0)
}

/// What a keyable property is worth on screen: its constant, or once it
/// carries keys, its ride's value at `at`.
fn shown(clip: &Clip, property: model::KeyProperty, at: f64) -> f64 {
    if clip.is_keyed(property) {
        clip.value_at(property, at)
    } else {
        clip.constant(property)
    }
}

/// Writes a keyable property the way the person means it: the constant
/// while the property has no keys, and once it rides, the key at `at` -
/// put on if there was none, with the ease of the key behind it. The engine
/// stops reading the constant the moment a property is keyed, so a knob or
/// a stage drag that wrote it would change nothing on screen.
fn write_keyable(clip: &mut Clip, property: model::KeyProperty, value: f64, at: f64) {
    if clip.is_keyed(property) {
        let ease = clip
            .keys_on(property)
            .rfind(|key| key.at < at)
            .map_or(model::KeyEase::LINEAR, |key| key.ease);
        clip.set_key(property, at, value, ease);
        return;
    }
    match property {
        model::KeyProperty::Scale => clip.scale = value,
        model::KeyProperty::OffsetX => clip.offset_x = value,
        model::KeyProperty::OffsetY => clip.offset_y = value,
        model::KeyProperty::Rotation => clip.rotation = value,
        model::KeyProperty::Opacity => clip.opacity = value,
        model::KeyProperty::Volume => clip.volume = value,
    }
}

/// The commands that turn `before`'s keys into `after`'s: a `SetClipKey`
/// for every key added or changed, a `ClearClipKey` for every key gone.
/// Two keys within a hair of each other are the same key.
fn key_commands(clip_id: &str, before: &Clip, after: &Clip) -> Vec<Command> {
    let near = |a: f64, b: f64| (a - b).abs() <= model::KEY_EPSILON;
    let mut commands = Vec::new();
    for property in model::KeyProperty::ALL {
        for key in after.keys_on(property) {
            let unchanged = before
                .keys_on(property)
                .any(|old| near(old.at, key.at) && old.value == key.value && old.ease == key.ease);
            if !unchanged {
                commands.push(Command::SetClipKey {
                    clip_id: clip_id.to_owned(),
                    property,
                    at: key.at,
                    value: key.value,
                    ease: key.ease,
                });
            }
        }
        for old in before.keys_on(property) {
            if !after.keys_on(property).any(|key| near(old.at, key.at)) {
                commands.push(Command::ClearClipKey {
                    clip_id: clip_id.to_owned(),
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
/// Every one can be keyed; `at` is the playhead's place in the clip, `0..=1`,
/// or None when it is outside - a keyed knob then shows its ride's value
/// there, and its cluster says whether a key sits under the playhead.
fn adjust_rows(chain: &[AppliedFilter], at: Option<f64>) -> Vec<AppliedParamData> {
    let Some(package) = Catalogue::builtin().get(ADJUST_ID) else {
        return Vec::new();
    };
    let held = chain.iter().find(|entry| entry.id == ADJUST_ID);
    package
        .manifest
        .params
        .iter()
        .map(|param| {
            let keyed = held.is_some_and(|entry| entry.is_keyed(&param.key));
            let (here, prev, next) = match (held, at) {
                (Some(entry), Some(at)) if keyed => {
                    let (prev, next) = entry.keys_around(&param.key, at);
                    (
                        entry.key_at(&param.key, at).is_some(),
                        prev.is_some(),
                        next.is_some(),
                    )
                }
                _ => (false, false, false),
            };
            let value = match (held, at) {
                (Some(entry), Some(at)) => entry.value_at(&param.key, at, param.default),
                (Some(entry), None) => entry
                    .params
                    .get(&param.key)
                    .copied()
                    .unwrap_or(param.default),
                (None, _) => param.default,
            };
            AppliedParamData {
                entry: -1,
                key: param.key.as_str().into(),
                label: t(&param.label).into(),
                group: t(&param.group).into(),
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

/// A chain as the inspector's stack draws it: one row per link, and one per
/// knob its package declares, holding the document's value or the default.
/// A link no package answers to keeps its row - so it can be removed - and
/// gets no knobs.
fn chain_rows(chain: &[AppliedFilter]) -> (Vec<AppliedEntryData>, Vec<AppliedParamData>) {
    let catalogue = Catalogue::builtin();
    let mut rows = Vec::new();
    let mut knobs = Vec::new();
    for (index, entry) in chain.iter().enumerate() {
        let package = catalogue
            .packages()
            .find(|package| package.answers_to(&entry.id));
        rows.push(AppliedEntryData {
            id: entry.id.as_str().into(),
            name: package
                .map(|package| package.manifest.effect.name.clone())
                .unwrap_or_else(|| label_of(&entry.id))
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
                label: t("Intensity").into(),
                group: "".into(),
                unit: "%".into(),
                min: 0.0,
                max: 100.0,
                step: 1.0,
                default_value: 100.0,
                value: entry
                    .params
                    .get(concat_effects::catalogue::INTENSITY)
                    .copied()
                    .unwrap_or(100.0) as f32,
                fmt: ParamFormat::Percent,
                keyable: false,
                keyed: false,
                here: false,
                prev: false,
                next: false,
            });
        }
        for param in &package.manifest.params {
            let step = if param.step > 0.0 {
                param.step
            } else {
                (param.max - param.min) / 200.0
            };
            knobs.push(AppliedParamData {
                entry: index as i32,
                key: param.key.as_str().into(),
                label: t(&param.label).into(),
                group: "".into(),
                unit: param.unit.as_str().into(),
                min: param.min as f32,
                max: param.max as f32,
                step: step as f32,
                default_value: param.default as f32,
                value: entry
                    .params
                    .get(&param.key)
                    .copied()
                    .unwrap_or(param.default) as f32,
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

fn label_of(id: &str) -> String {
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
        // The words first, so everything published from here on is in
        // the remembered language.
        i18n::select(prefs.locale.as_deref().unwrap_or(i18n::ENGLISH), &host.dirs);
        let languages = i18n::languages(&host.dirs);
        let recents = projects::list(&host.dirs.config);
        let text_presets = presets::all(&host.dirs);
        let mut studio = Self {
            prefs,
            library: Default::default(),
            session: None,
            echo: None,
            empty: Project::new(),
            dirty: false,
            autosave: slint::Timer::default(),
            media: crate::panes::media_bin::MediaBin::default(),
            peaks: HashMap::new(),
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
            playhead: 0.0,
            // ~20px a second: a ten-second cut fits a pane at its default width.
            playing: false,
            transport: slint::Timer::default(),
            hover: None,
            hover_hush: slint::Timer::default(),
            clipboard: None,
            monitor: crate::panes::monitor::MonitorPane::default(),
            export: Default::default(),
            settings: crate::panes::settings::SettingsPane::default(),
            relink: crate::panes::relink::RelinkPane::default(),
            open_menu: -1,
            menu_bar_token: 0,
            menu_target: None,
            menu_media: None,
            menu_token: 0,
            toast: ToastState::default(),
            on_start: true,
            start: crate::panes::start::StartPane::default(),
            recents,
            posters: HashMap::new(),
            posters_pending: HashSet::new(),
            project_name: "Untitled project".into(),
            dock: default_dock(),
            dock_aside: crate::dock::compact_dock(),
            compact: false,
            workspace: (0.0, 0.0),
            divider_press: None,
            gesture: Gesture::None,
            stage_guides: Vec::new(),
            inspector_jump: (0, "", ""),
            audition: None,
            revision: 0,
            flat: None,
            commit_pending: false,
            commit_target: None,
            commit_timer: slint::Timer::default(),
            shelf_stamp: std::cell::RefCell::new(None),
            look_art: std::cell::RefCell::new(HashMap::new()),
            packages_watch: slint::Timer::default(),
            packages_seen: 0,
            packages_pending: None,
            last_commit: None,
            title_blocks: HashMap::new(),
            drop: None,
            project_sheet: crate::panes::project::ProjectPane::default(),
            captions: crate::panes::captions::CaptionsPane::default(),
            speech: crate::panes::speech::SpeechPane::default(),
            text_presets,
            languages,
            brush: 0,
            brush_size: 0.06,
            painting: true,
            cutout_jobs: HashMap::new(),
            enhance_jobs: HashMap::new(),
            region_job: None,
            pending_stroke: None,
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

    // ── reading the edit ──

    /// The project as the window should draw it: the echo while a gesture
    /// runs, the session's otherwise, and nothing at all on the launch screen.
    pub fn project(&self) -> &Project {
        self.echo
            .as_ref()
            .or_else(|| self.session.as_ref().map(|session| session.project()))
            .unwrap_or(&self.empty)
    }

    pub fn timeline(&self) -> &Timeline {
        self.project().active()
    }

    pub fn clip(&self, id: &str) -> Option<&Clip> {
        self.timeline().clip(id)
    }

    /// The active timeline's frame rate. With no project open, the empty
    /// project's one timeline answers, at the default.
    pub fn frame_rate(&self) -> f32 {
        self.project().active().video.rate() as f32
    }

    /// The active timeline's output size.
    pub fn output_size(&self) -> (u32, u32) {
        let video = self.project().active().video;
        (video.width, video.height)
    }

    /// The monitor's quality tier for the active timeline: 0 full, 1 half,
    /// 2 quarter; see `MonitorPane::quality_of`.
    pub fn quality_of(&self) -> usize {
        self.monitor.quality_of(self.project())
    }

    /// The track a row index names. Rows count from the top of the panel and
    /// the model stores lanes bottom-most first - the compositing order - so
    /// this is the one place the two orders meet.
    pub fn row_track(&self, row: i32) -> Option<&Track> {
        let tracks = &self.timeline().tracks;
        let count = tracks.len() as i32;
        if row < 0 || row >= count {
            return None;
        }
        tracks.get((count - 1 - row) as usize)
    }

    pub fn row_of(&self, track_id: &str) -> i32 {
        let tracks = &self.timeline().tracks;
        tracks
            .iter()
            .position(|track| track.id == track_id)
            .map(|index| tracks.len() as i32 - 1 - index as i32)
            .unwrap_or(0)
    }

    pub fn locked(&self, track_id: &str) -> bool {
        self.lanes
            .lane_view
            .get(track_id)
            .is_some_and(|view| view.locked)
    }

    fn lane_size(&self, track_id: &str) -> TrackSize {
        self.lanes
            .lane_view
            .get(track_id)
            .map_or(TrackSize::Auto, |view| view.size)
    }

    /// How tall one lane is drawn: a lane takes the height of the tallest
    /// thing on it, without the lanes having to be typed. An empty one takes
    /// the middle size.
    fn lane_height(&self, lane: &Track) -> f32 {
        match self.lane_size(&lane.id) {
            TrackSize::Small => LANE_SMALL,
            TrackSize::Medium => LANE_MEDIUM,
            TrackSize::Large => LANE_LARGE,
            TrackSize::Auto => {
                let tallest = self
                    .timeline()
                    .clips
                    .iter()
                    .filter(|clip| clip.track_id == lane.id)
                    .map(|clip| match clip.kind {
                        model::ClipKind::Video | model::ClipKind::Image => LANE_LARGE,
                        model::ClipKind::Audio => LANE_MEDIUM,
                        // A title is its name strip alone; a layer has no
                        // picture at all. Neither needs a body's height.
                        model::ClipKind::Text | model::ClipKind::Layer => LANE_SMALL,
                    })
                    .fold(0.0_f32, f32::max);
                if tallest > 0.0 { tallest } else { LANE_MEDIUM }
            }
        }
    }

    /// Every lane's height, top-most first.
    pub fn lane_heights(&self) -> Vec<f32> {
        self.timeline()
            .tracks
            .iter()
            .rev()
            .map(|lane| self.lane_height(lane))
            .collect()
    }

    pub fn row_at(&self, y: f32) -> i32 {
        row_at(&self.lane_heights(), y)
    }

    /// Seconds the project runs to, for Fit and for the scroll floor.
    pub fn duration(&self) -> f32 {
        self.timeline().clips.iter().fold(0.0_f64, |longest, clip| {
            longest.max(clip.start + clip.duration)
        }) as f32
    }

    /// Clip edges, the playhead and zero all pull, and the nearest inside
    /// the threshold wins.
    pub fn snapped(&self, time: f32, threshold: f32, exclude: &str) -> f32 {
        if !self.lanes.snap {
            return time;
        }
        let mut best = time;
        let mut best_distance = threshold;
        let mut consider = |target: f32| {
            let distance = (target - time).abs();
            if distance < best_distance {
                best = target;
                best_distance = distance;
            }
        };
        consider(0.0);
        consider(self.playhead);
        for clip in &self.timeline().clips {
            if clip.id == exclude {
                continue;
            }
            consider(clip.start as f32);
            consider((clip.start + clip.duration) as f32);
        }
        best
    }

    // ── changing the edit ──

    /// Applies one command to the session and reports the id it minted.
    /// A refusal becomes a notice; the echo, if any, is dropped either way,
    /// because the session's project is the truth again.
    pub fn apply(&mut self, command: Command) -> Option<String> {
        self.flush_commit();
        self.echo = None;
        // Anything but an inspector commit ends the coalescing window; the
        // commit path sets `last_commit` again right after calling here.
        self.last_commit = None;
        let session = self.session.as_mut()?;
        match session.apply(command) {
            Ok(view) => {
                self.after_change();
                view.created_id
            }
            Err(error) => {
                self.notify(&error, true);
                None
            }
        }
    }

    /// [`Studio::apply`] as one move of an inspector gesture: the same
    /// bookkeeping, but the editor folds it into the gesture's undo step
    /// and the coalescing window stays open for the next move.
    fn apply_within(&mut self, gesture: &str, command: Command) {
        self.echo = None;
        let Some(session) = self.session.as_mut() else {
            return;
        };
        match session.apply_within(Some(gesture), command) {
            Ok(_) => self.after_change(),
            Err(error) => self.notify(&error, true),
        }
    }

    /// The bookkeeping every change to the edit needs: the caches that
    /// follow the document, the autosave, the monitor and the mix.
    fn after_change(&mut self) {
        self.dirty = true;
        self.revision += 1;
        self.assign_media_rows();
        let survivors: HashSet<String> = self
            .timeline()
            .clips
            .iter()
            .map(|clip| clip.id.clone())
            .collect();
        self.selection.retain(|id| survivors.contains(id));
        self.schedule_autosave();
        self.sync_audio();
        self.request_media_art();
        self.request_preview();
        self.ensure_cutouts();
        self.ensure_regions();
    }

    pub fn undo(&mut self) {
        self.flush_commit();
        self.echo = None;
        if let Some(session) = self.session.as_mut()
            && session.can_undo()
        {
            session.undo();
            self.after_change();
        }
    }

    pub fn redo(&mut self) {
        self.flush_commit();
        self.echo = None;
        if let Some(session) = self.session.as_mut()
            && session.can_redo()
        {
            session.redo();
            self.after_change();
        }
    }

    /// Starts a gesture's preview: the echo is a clone of the project the
    /// pointer will mutate.
    pub fn begin_echo(&mut self) {
        if self.echo.is_none() {
            self.echo = Some(self.project().clone());
        }
    }

    pub fn echo_clip_mut(&mut self, id: &str) -> Option<&mut Clip> {
        self.echo.as_mut()?.active_mut().clip_mut(id)
    }

    fn schedule_autosave(&mut self) {
        // A second and a half after the last change, not after the first:
        // a drag that commits ten edits saves once.
        self.autosave.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_millis(1500),
            || {
                crate::host::Shell::with(|shell, app| {
                    shell.studio.borrow_mut().save(false);
                    shell.studio.borrow().publish(&app, &shell.models);
                });
            },
        );
    }

    /// Writes the document, on a worker. `announce` says so on success;
    /// a failure is always said.
    pub fn save(&mut self, announce: bool) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        self.autosave.stop();
        let (path, document) = session.prepare_save(None);
        self.dirty = false;
        spawn(
            move || projects::save(&path, &document),
            move |studio, _, _, result| match result {
                Ok(()) if announce => studio.notify(&t("Project saved"), false),
                Ok(()) => {}
                Err(error) => {
                    studio.dirty = true;
                    studio.notify(&tf("Could not save: {0}", &[&error]), true);
                }
            },
        );
    }

    /// The audible clip set, handed to playback whenever the edit changes.
    fn sync_audio(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let clips = session.flattened_clips();
        let specs: Vec<ClipSpec> = clips
            .iter()
            .filter(|clip| {
                !clip.muted
                    && (clip.kind == concat_export::ClipKind::Audio
                        || (clip.kind == concat_export::ClipKind::Video
                            && clip.has_audio.unwrap_or(true)))
            })
            // Through the exporter's own cut into pieces, so a curve or a
            // reverse sounds in the window as it will in the file.
            .flat_map(concat_export::audio_pieces)
            .map(|piece| ClipSpec {
                path: piece.path.to_string_lossy().into_owned(),
                audio_stream: piece.stream.map(|index| index as u32),
                start: piece.start,
                duration: piece.duration,
                source_start: piece.source_start,
                volume: piece.volume as f32,
                volume_curve: piece
                    .volume_curve
                    .keys()
                    .iter()
                    .map(|key| concat_host::playback::GainKey {
                        at: key.at,
                        gain: key.value,
                        ease: [key.ease.x1, key.ease.y1, key.ease.x2, key.ease.y2],
                    })
                    .collect(),
                fade_in: piece.fade_in,
                fade_out: piece.fade_out,
                speed: piece.speed,
                preserve_pitch: piece.preserve_pitch,
                chain: piece.filter_chain,
            })
            .collect();
        self.host
            .playback
            .set_clips(std::path::PathBuf::from(session.path()), specs);
    }

    // ── the monitor ──

    /// Asks the monitor for the frame at the playhead; see
    /// `MonitorPane`.
    pub fn request_preview(&mut self) {
        self.handle(crate::panes::Msg::Monitor(
            crate::panes::monitor::MonitorMsg::Request,
        ));
    }

    /// What the monitor draws at the playhead: the document flattened with
    /// its titles, plus the frame's own additions - a look being shown, a
    /// cutout being painted - and the session's settings. None without a
    /// project.
    pub fn preview_clips(
        &mut self,
    ) -> Option<(
        std::sync::Arc<Vec<concat_export::ExportClip>>,
        concat_project::DocumentSettings,
    )> {
        let session = self.session.as_ref()?;
        // The echo when there is one: a picture being dragged on the stage
        // is drawn where the pointer has it, not where the document last
        // had it. Same flattening the session does for itself, project
        // folder included: that is what names a cutout's masks, and
        // without it the monitor would show every cutout as shot.
        let project_dir = std::path::PathBuf::from(session.path());
        let (width, height) = self.output_size();
        // The flattening is the document's, not the frame's: kept between
        // frames of an unchanged document and handed over by pointer, so
        // playback and scrubbing neither flatten nor plan again. A gesture
        // in flight - the echo - is not the document, and flattens fresh.
        let cached = self.echo.is_none()
            && self
                .flat
                .as_ref()
                .is_some_and(|(at, w, h, _)| *at == self.revision && *w == width && *h == height);
        let clips = if cached {
            std::sync::Arc::clone(&self.flat.as_ref().expect("checked").3)
        } else {
            let mut clips = concat_export::flatten::flatten_timeline_in(
                self.project(),
                None,
                Some(&project_dir),
            );
            // Titles, painted to pictures and rejoined; see concat-host's titles.
            if self.echo.is_some() {
                // A change in flight: the titles are painted in memory at
                // the monitor's size and handed to it as pixels - no file
                // per pointer step - and the blocks they report are scaled
                // back up to the output's terms, which the stage measures
                // in. Nothing is written until the change is committed.
                let (shown_w, shown_h) = crate::panes::monitor::MonitorPane::frame_size(
                    self.quality_of(),
                    (width, height),
                );
                let scale = f64::from(shown_w) / f64::from(width);
                let up = |px: u32| (f64::from(px) / scale).round() as u32;
                let up_off = |px: i32| (f64::from(px) / scale).round() as i32;
                for title in self
                    .host
                    .titles
                    .clips_live(self.project(), shown_w, shown_h)
                {
                    self.title_blocks.insert(
                        title.clip_id,
                        (
                            (up(title.block.0), up(title.block.1)),
                            (up_off(title.offset.0), up_off(title.offset.1)),
                        ),
                    );
                    if let Some(frame) = title.frame {
                        self.host
                            .monitor
                            .hold_still(std::path::Path::new(&title.clip.path), frame);
                    }
                    clips.push(title.clip);
                }
            } else {
                for title in self.host.titles.clips(self.project(), width, height) {
                    self.title_blocks
                        .insert(title.clip_id, (title.block, title.offset));
                    clips.push(title.clip);
                }
            }
            let clips = std::sync::Arc::new(clips);
            if self.echo.is_none() {
                self.flat = Some((self.revision, width, height, std::sync::Arc::clone(&clips)));
            }
            clips
        };
        // The frame's own additions - a look being shown, a cutout being
        // painted - go on a copy, so the kept list stays the document's.
        let mut own: Option<Vec<concat_export::ExportClip>> = None;
        // The look being shown before it is laid down goes into this
        // frame only, as the layer it would be: over every track, the
        // whole way along, at full strength. The timeline is as it was.
        if let Some(filter_id) = self.audition.clone() {
            let effects = vec![AppliedFilter::new(filter_id)];
            let video_filter_chain = concat_export::chains::video_effect_chain(&effects);
            let track = clips
                .iter()
                .map(|flat| flat.track)
                .max()
                .map_or(0, |top| top + 1);
            own.get_or_insert_with(|| (*clips).clone())
                .push(concat_export::ExportClip {
                    muted: true,
                    volume: 0.0,
                    effects,
                    video_filter_chain,
                    has_audio: Some(false),
                    ..concat_export::ExportClip::blank(
                        concat_export::ClipKind::Layer,
                        0.0,
                        f64::from(self.duration()).max(1.0),
                        track,
                    )
                });
        }
        // While the brushes are out, the clip being painted is drawn with
        // its cutout tinted over the whole picture rather than cut, so a
        // stroke shows what it grabbed against what it left.
        if self.painting
            && let Some(target) = self.paint_target()
            && let Some(media) = self.project().media_by_id(&target.media_id)
        {
            let (path, start) = (media.path.clone(), target.start);
            if let Some(flat) = own
                .get_or_insert_with(|| (*clips).clone())
                .iter_mut()
                .find(|flat| flat.path == path && (flat.start - start).abs() < 1e-6)
            {
                flat.highlighted = true;
            }
        }
        let clips = own.map(std::sync::Arc::new).unwrap_or(clips);
        Some((clips, session.settings()))
    }

    // ── playback ──

    pub fn play_toggle(&mut self) {
        if self.playing {
            self.pause();
            return;
        }
        if self.session.is_none() {
            return;
        }
        // Playing from the tail would sit there doing nothing, so the
        // button rewinds first - what every editor does.
        if self.playhead >= self.duration() {
            self.playhead = 0.0;
        }
        // The transport owns the monitor from here; a frame from under a
        // pointer that happens to be over the lanes must not stay on it.
        self.end_hover();
        self.playing = true;
        self.host.playback.play(f64::from(self.playhead));
        // The clock is the audio device's; this follows it at 30 Hz and
        // asks the monitor for the frame under it each time.
        self.transport.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(33),
            || {
                crate::host::Shell::with(|shell, app| {
                    {
                        let mut studio = shell.studio.borrow_mut();
                        let end = studio.duration();
                        let position = studio.host.playback.position() as f32;
                        studio.playhead = position.min(end);
                        // The view follows: a playhead that runs off the
                        // right edge, or sits off the left, pages the lanes
                        // to it, the way every editor keeps the cut in view.
                        if let Some((low, high)) = studio.lanes.published_span() {
                            let screen = (high - low) / 3.0;
                            let left = studio.lanes.scroll_left;
                            if screen > 0.0
                                && (studio.playhead > left + screen * 0.95
                                    || studio.playhead < left)
                            {
                                studio.lanes.scroll_left =
                                    (studio.playhead - screen * 0.05).max(0.0);
                            }
                        }
                        if position >= end {
                            studio.pause();
                        } else {
                            studio.request_preview();
                        }
                    }
                    shell.studio.borrow().publish_lanes(&app, &shell.models);
                });
            },
        );
    }

    pub fn pause(&mut self) {
        self.playing = false;
        self.transport.stop();
        self.host.playback.pause();
    }

    /// Moves the playhead, the transport with it, and asks for the frame.
    /// Never before zero; past the end of the content only when Settings
    /// lets it, which is the default, so the ruler can be clicked beyond the
    /// last clip and something placed at the playhead there.
    pub fn seek(&mut self, seconds: f32) {
        self.playhead = seconds.max(0.0);
        if self.prefs.playhead_stops_at_end {
            self.playhead = self.playhead.min(self.duration().max(0.0));
        }
        self.host.playback.seek(f64::from(self.playhead));
        self.request_preview();
    }

    // ── the preview axis ──

    /// The instant the monitor shows: the pointer's while it crosses the
    /// lanes with the preview axis on, else the playhead's.
    pub fn preview_time(&self) -> f32 {
        self.hover.unwrap_or(self.playhead)
    }

    /// The pointer is over the lanes at `seconds`. Nothing while playing -
    /// the transport owns the monitor then - and nothing with the axis off.
    pub fn hover(&mut self, seconds: f32) {
        if !self.prefs.preview_axis || self.playing {
            return;
        }
        let seconds = seconds.max(0.0);
        if self.hover == Some(seconds) {
            return;
        }
        self.hover = Some(seconds);
        if self.prefs.preview_axis_audio {
            // A burst of the sound under the pointer: the engine plays from
            // there and is stopped a beat later, then put back where the
            // playhead is, so the next Play starts where the cut says.
            self.host.playback.play(f64::from(seconds));
            let playhead = f64::from(self.playhead);
            self.hover_hush.start(
                slint::TimerMode::SingleShot,
                std::time::Duration::from_millis(120),
                move || {
                    crate::host::Shell::with(|shell, _| {
                        let studio = shell.studio.borrow();
                        if !studio.playing {
                            studio.host.playback.pause();
                            studio.host.playback.seek(playhead);
                        }
                    });
                },
            );
        }
        self.request_preview();
    }

    /// The pointer left the lanes, or the axis went off: the monitor is the
    /// playhead's again.
    pub fn end_hover(&mut self) {
        if self.hover.take().is_some() {
            self.request_preview();
        }
    }

    // ── the bin ──

    /// Gives every media item the row Slint knows it by; see
    /// `MediaBin::assign_rows`.
    fn assign_media_rows(&mut self) {
        let mut bin = std::mem::take(&mut self.media);
        bin.assign_rows(self.project());
        self.media = bin;
    }

    /// Decodes art for every media item that has none yet: its pictures
    /// and the waveform of its default audio stream - and, for every clip
    /// that plays another of its media's streams, that stream's waveform
    /// too, so a lane shows the sound it will make.
    pub(crate) fn request_media_art(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project_path = session.path().to_owned();
        // A file larger than HD gets a proxy for playback and the
        // filmstrips, written once on the scheduler's proxy lane; see
        // concat_host::proxy. Asked once per file per project.
        let unproxied: Vec<(String, String, u32, u32, Option<concat_media::ColorRange>)> = self
            .project()
            .media
            .iter()
            .filter(|item| item.kind == model::MediaKind::Video && !item.placeholder)
            .filter(|item| !self.proxied.contains(&item.id))
            .filter_map(|item| {
                let (width, height) = (item.width?, item.height?);
                Some((
                    item.id.clone(),
                    item.path.clone(),
                    width,
                    height,
                    item.color_range.map(concat_export::engine_range),
                ))
            })
            .collect();
        for (id, path, width, height, range) in unproxied {
            concat_host::proxy::ensure(
                std::path::Path::new(&project_path),
                &path,
                width,
                height,
                range,
            );
            self.proxied.insert(id);
        }
        /// One job: the art key it fills, and what to decode.
        struct Want {
            key: String,
            id: String,
            path: String,
            kind: model::MediaKind,
            has_audio: bool,
            duration: Option<f64>,
            stream: Option<u32>,
            pictures: bool,
            /// The levels the file is read as, so the filmstrip reads the
            /// proxy written for that reading.
            range: Option<concat_media::ColorRange>,
        }
        let project = self.project();
        let mut wanted: Vec<Want> = project
            .media
            .iter()
            .filter(|item| !item.placeholder && !item.path.is_empty())
            .filter(|item| {
                !self.art_pending.contains(&item.id) && !self.art_failed.contains(&item.id)
            })
            .filter(|item| {
                let needs_thumb = item.kind != model::MediaKind::Audio
                    && (!self.media.thumbs.contains_key(&item.id)
                        || !self.strips.contains_key(&item.id));
                let needs_peaks = (item.kind == model::MediaKind::Audio || item.has_audio)
                    && !self.peaks.contains_key(&item.id);
                needs_thumb || needs_peaks
            })
            .map(|item| Want {
                key: item.id.clone(),
                id: item.id.clone(),
                path: item.path.clone(),
                kind: item.kind,
                has_audio: item.has_audio,
                duration: item.duration,
                stream: None,
                pictures: item.kind != model::MediaKind::Audio,
                range: item.color_range.map(concat_export::engine_range),
            })
            .collect();
        // The other streams clips have chosen, once each.
        let mut asked: HashSet<String> = wanted.iter().map(|want| want.key.clone()).collect();
        for clip in project
            .timelines
            .iter()
            .flat_map(|timeline| timeline.clips.iter())
        {
            let Some(stream) = clip.audio_stream else {
                continue;
            };
            let Some(item) = project.media_by_id(&clip.media_id) else {
                continue;
            };
            if item.placeholder
                || item.path.is_empty()
                || !(item.kind == model::MediaKind::Audio || item.has_audio)
            {
                continue;
            }
            let key = art_key(&item.id, Some(stream));
            if self.peaks.contains_key(&key)
                || self.art_pending.contains(&key)
                || self.art_failed.contains(&key)
                || !asked.insert(key.clone())
            {
                continue;
            }
            wanted.push(Want {
                key,
                id: item.id.clone(),
                path: item.path.clone(),
                kind: item.kind,
                has_audio: item.has_audio,
                duration: item.duration,
                stream: Some(stream),
                pictures: false,
                range: item.color_range.map(concat_export::engine_range),
            });
        }
        for want in wanted {
            let Want {
                key,
                id,
                path,
                kind,
                has_audio,
                duration,
                stream,
                mut pictures,
                range,
            } = want;

            // Restore pictures from the project's JPEG artwork cache before
            // starting a decoder. Peaks already have their own disk cache in
            // concat-media; thumbnails and filmstrips are what used to be
            // recreated on every project open.
            if stream.is_none() && kind != model::MediaKind::Audio {
                let cached = cached_media_art(&project_path, &id, &path, kind);
                if let Some(image) = cached.thumbnail {
                    self.media.thumbs.insert(id.clone(), image);
                }
                if let Some(strip) = cached.strip {
                    self.strips.insert(id.clone(), strip.into());
                }
                pictures = !self.media.thumbs.contains_key(&id) || !self.strips.contains_key(&id);
            }

            let needs_peaks =
                (kind == model::MediaKind::Audio || has_audio) && !self.peaks.contains_key(&key);
            if !pictures && !needs_peaks {
                continue;
            }

            self.art_pending.insert(key);
            let project = project_path.clone();
            // On the artwork lane, a few at a time - see host.rs.
            spawn_art(
                move || {
                    media_art(
                        id, path, kind, has_audio, duration, project, stream, pictures, range,
                    )
                },
                |studio, _, _, art: MediaArt| {
                    let key = art_key(&art.id, art.stream);
                    studio.art_pending.remove(&key);
                    if art.thumbnail.is_none() && art.strip.is_none() && art.peaks.is_none() {
                        studio.art_failed.insert(key.clone());
                    }
                    if let Some(frame) = art.thumbnail {
                        studio.media.thumbs.insert(art.id.clone(), image_of(&frame));
                    }
                    if let Some((frame, frames)) = art.strip {
                        studio
                            .strips
                            .insert(art.id.clone(), Strip::of(&frame, frames));
                    }
                    if let Some(peaks) = art.peaks {
                        let prefix = format!("{key}|");
                        studio.peaks.insert(key, peaks);
                        studio
                            .waves
                            .borrow_mut()
                            .retain(|key, _| !key.starts_with(&prefix));
                    }
                },
            );
        }
        self.request_window_art();
    }

    /// Decodes a cell strip for every picture clip on the current timeline
    /// whose cut is too short a piece of its footage for the file's own
    /// strip - see `host::strip_window` - and lets go of the cells nothing
    /// shows once there are more than `WINDOWS_KEPT` of them.
    fn request_window_art(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project_path = session.path().to_owned();
        struct Want {
            key: String,
            id: String,
            path: String,
            level: u32,
            cell: u32,
            duration: f64,
        }
        let mut shown: HashSet<String> = HashSet::new();
        let mut wanted: Vec<Want> = Vec::new();
        for clip in &self.timeline().clips {
            if clip.kind != model::ClipKind::Video {
                continue;
            }
            let Some(item) = self.project().media_by_id(&clip.media_id) else {
                continue;
            };
            if item.placeholder || item.path.is_empty() {
                continue;
            }
            let Some((start, span, duration)) = self.cut_of(clip) else {
                continue;
            };
            let Some((level, cell)) = strip_window(start, span, duration) else {
                continue;
            };
            let key = window_key(&clip.media_id, level, cell);
            if !shown.insert(key.clone())
                || self.windows.contains_key(&key)
                || self.window_pending.contains(&key)
            {
                continue;
            }
            wanted.push(Want {
                key,
                id: item.id.clone(),
                path: item.path.clone(),
                level,
                cell,
                duration,
            });
        }
        if self.windows.len() > WINDOWS_KEPT {
            self.windows.retain(|key, _| shown.contains(key));
        }
        for want in wanted {
            let Want {
                key,
                id,
                path,
                level,
                cell,
                duration,
            } = want;
            if let Some(strip) = cached_window_art(&project_path, &id, &path, level, cell) {
                self.windows.insert(key, strip.into());
                continue;
            }
            self.window_pending.insert(key);
            let project = project_path.clone();
            spawn_strip(
                move || window_art(id, path, project, level, cell, duration),
                |studio, _, _, art: WindowArt| {
                    let key = window_key(&art.id, art.level, art.cell);
                    studio.window_pending.remove(&key);
                    if let Some((frame, frames)) = art.strip {
                        studio.windows.insert(key, Strip::of(&frame, frames));
                    }
                },
            );
        }
    }

    /// A picture clip's cut as fractions of its footage - where it begins
    /// and how much it covers - with the footage's length in seconds.
    /// `None` for a still or footage of unknown length: one frame, all of it.
    fn cut_of(&self, clip: &Clip) -> Option<(f64, f64, f64)> {
        let seconds = self
            .project()
            .media
            .iter()
            .find(|item| item.id == clip.media_id)
            .and_then(|item| item.duration)
            .filter(|seconds| *seconds > 0.0)?;
        Some((
            (clip.source_start / seconds).clamp(0.0, 1.0),
            (clip.duration * clip.speed / seconds).clamp(0.0, 1.0),
            seconds,
        ))
    }

    /// The clip's filmstrip, with the window of the strip its cut covers:
    /// where in the footage it starts and how much of the footage it spans,
    /// both as fractions, so the lane can pick the frame under each tile
    /// without knowing about speed or source time.
    fn strip_of(&self, clip: &Clip) -> StripData {
        if !matches!(clip.kind, model::ClipKind::Video | model::ClipKind::Image) {
            return StripData::default();
        }
        let Some(mut strip) = self.strips.get(&clip.media_id) else {
            return StripData::default();
        };
        let (mut start, mut span) = (0.0, 1.0);
        if let Some((cut_start, cut_span, duration)) = self.cut_of(clip) {
            start = cut_start;
            span = cut_span;
            // The cell strip, once it is here: the cut re-expressed as a
            // window of that cell rather than of the whole file. Until it
            // arrives the file's strip stands in, and the tiles repeat.
            if let Some((level, cell)) = strip_window(start, span, duration)
                && let Some(window) = self.windows.get(&window_key(&clip.media_id, level, cell))
            {
                let (cell_start, cell_span) = (window_start(level, cell), window_span(level));
                start = ((start - cell_start) / cell_span).clamp(0.0, 1.0);
                span = (span / cell_span).clamp(0.0, 1.0);
                strip = window;
            }
        }
        StripData {
            image: strip.image.clone(),
            frames: strip.frames,
            frame_width: strip.frame_width,
            height: strip.height,
            start: start as f32,
            span: span as f32,
        }
    }

    /// The memoised envelope for one clip, quantised to a thirtieth of a
    /// second so a trim revisits a handful of entries rather than minting
    /// one per pointer event, and to a step of columns so a zoom rebuilds
    /// it at each step of that and not at every pixel. A sound clip's
    /// whole body, a picture clip's band under its frames; a picture that
    /// is muted, or whose file has no sound, shows the empty band.
    ///
    /// At unity: the clip's volume scales the drawing in the lane, so a
    /// volume drag never comes here - see `format::wave_path`.
    ///
    /// Only the window of the clip that is on screen, and a screen either
    /// side of it, is built: what comes back is the path and where it
    /// sits on the clip, as fractions of the clip's length. A clip zoomed
    /// in far enough to be wider than the screen many times over would
    /// otherwise spread its bars across all of that width - a bar the
    /// width of a finger - or want tens of thousands of them. This way a
    /// bar is one pitch on screen at every zoom, and the count is bounded
    /// by the screen, not the clip. The window is quantised to strides
    /// of sixty-four bars, so a scroll rebuilds at each stride and not at
    /// every pixel, and the cache holds a few hundred windows before it
    /// is emptied.
    fn wave(&self, clip: &Clip) -> (SharedString, f32, f32) {
        if clip.muted == Some(true) {
            return Default::default();
        }
        // The stream this clip plays; its peaks come when they are decoded,
        // and until then the lane is bare rather than showing another
        // track's shape.
        let art = art_key(&clip.media_id, clip.audio_stream);
        let Some(peaks) = self.peaks.get(&art) else {
            return Default::default();
        };
        let step = |seconds: f32| (seconds * WAVE_STEPS).round() / WAVE_STEPS;
        let (source_start, duration) = (step(clip.source_start as f32), step(clip.duration as f32));
        if duration <= 0.0 {
            return Default::default();
        }
        let seconds_per_pixel = self.lanes.seconds_per_pixel;
        let stride = (WAVE_PITCH * seconds_per_pixel * 64.0).max(1.0 / WAVE_STEPS);
        let (from, to) = match self.lanes.published_span() {
            Some((left, right)) => {
                let from = (left - clip.start as f32).max(0.0);
                let to = (right - clip.start as f32).min(duration);
                if to > from {
                    (
                        (from / stride).floor() * stride,
                        ((to / stride).ceil() * stride).min(duration),
                    )
                } else {
                    (0.0, duration)
                }
            }
            None => (0.0, duration),
        };
        let window = to - from;
        let columns = wave_columns(window, seconds_per_pixel);
        let placed = (from / duration, window / duration);
        let key = format!("{art}|{source_start:.3}|{from:.3}|{window:.3}|{columns}");
        if let Some(cached) = self.waves.borrow().get(&key) {
            return (cached.clone(), placed.0, placed.1);
        }
        let built = SharedString::from(wave_path(
            peaks,
            source_start + from,
            window,
            columns,
            WAVE_BAR,
        ));
        let mut waves = self.waves.borrow_mut();
        if waves.len() >= 512 {
            waves.clear();
        }
        waves.insert(key, built.clone());
        (built, placed.0, placed.1)
    }

    // ── placing things ──

    /// What a library payload names - "media:12", "text:default:Title" -
    /// before the timeline has decided where to put it.
    pub fn incoming(&self, payload: &str) -> Option<DropPlan> {
        let mut fields = payload.splitn(3, ':');
        let (sort, id) = (fields.next()?, fields.next()?);
        let label = fields.next().unwrap_or(id);
        match sort {
            "media" => {
                let item = self.media.by_row(self.project(), id.parse().ok()?)?;
                Some(DropPlan {
                    kind: match item.kind {
                        model::MediaKind::Audio => ClipKind::Audio,
                        model::MediaKind::Image => ClipKind::Image,
                        model::MediaKind::Video => ClipKind::Video,
                    },
                    label: item.name.clone(),
                    media: item.id.clone(),
                    start: 0.0,
                    // A still has no length of its own; a file with no stated
                    // duration gets the engine's own fallback.
                    duration: if item.kind == model::MediaKind::Image {
                        LAYER_DURATION
                    } else {
                        item.duration.unwrap_or(5.0) as f32
                    }
                    .max(MIN_DURATION),
                    row: 0,
                })
            }
            // A title: the preset's id rides where a file's media id would,
            // "default" for the plain one.
            "text" => Some(DropPlan {
                kind: ClipKind::Text,
                label: label.to_owned(),
                media: id.to_owned(),
                start: 0.0,
                duration: LAYER_DURATION,
                row: 0,
            }),
            // A look dragged from the Filters page: a layer over a span.
            // The package id rides in `media`, there being no file.
            "filter" => Some(DropPlan {
                kind: ClipKind::Filter,
                label: label.to_owned(),
                media: id.to_owned(),
                start: 0.0,
                duration: LAYER_DURATION,
                row: 0,
            }),
            _ => None,
        }
    }

    /// A drag over the lanes: the pointer names the moment and the lane.
    pub fn plan(&self, payload: &str, seconds: f32, row: i32) -> Option<DropPlan> {
        let mut plan = self.incoming(payload)?;
        let lanes = self.timeline().tracks.len() as i32;
        if lanes == 0 {
            return None;
        }
        plan.row = row.clamp(0, lanes - 1);
        if self
            .row_track(plan.row)
            .is_none_or(|track| self.locked(&track.id))
        {
            return None;
        }
        plan.start = self
            .snapped(seconds.max(0.0), 8.0 * self.lanes.seconds_per_pixel, "")
            .max(0.0);
        Some(plan)
    }

    /// Commits a plan that has a lane, and selects what it made.
    pub fn place(&mut self, plan: &DropPlan) {
        let Some(track_id) = self.row_track(plan.row).map(|track| track.id.clone()) else {
            return;
        };
        let created = if plan.kind == ClipKind::Text {
            self.add_title(
                Some(track_id),
                f64::from(plan.start),
                f64::from(plan.duration),
                &plan.media,
            )
        } else if plan.kind == ClipKind::Filter {
            self.apply(Command::AddLayerClip {
                track_id: Some(track_id),
                start: f64::from(plan.start),
                duration: Some(f64::from(plan.duration)),
                effect_id: plan.media.clone(),
                name: plan.label.clone(),
            })
        } else {
            self.apply(Command::AddClip {
                media_id: plan.media.clone(),
                track_id,
                start: f64::from(plan.start),
                ripple: true,
            })
        };
        if let Some(id) = created {
            self.selection = vec![id];
        }
    }

    /// A card clicked rather than dragged: the playhead names the moment
    /// and the engine finds a lane with room.
    pub fn place_at_playhead(&mut self, payload: &str) {
        let Some(plan) = self.incoming(payload) else {
            return;
        };
        let start = f64::from(self.playhead.max(0.0));
        let created = if plan.kind == ClipKind::Text {
            self.add_title(None, start, f64::from(plan.duration), &plan.media)
        } else if plan.kind == ClipKind::Filter {
            self.apply(Command::AddLayerClip {
                track_id: None,
                start,
                duration: Some(f64::from(plan.duration)),
                effect_id: plan.media.clone(),
                name: plan.label.clone(),
            })
        } else {
            self.apply(Command::AddClipAtFirstFree {
                media_id: plan.media.clone(),
                start,
            })
        };
        if let Some(id) = created {
            self.selection = vec![id];
        }
    }

    /// Places a title in the look a preset names - "default" for the plain
    /// title - and, when the preset brings a font, registers the font on
    /// the project in the same step, so the painter finds it the first
    /// time it draws the words.
    fn add_title(
        &mut self,
        track_id: Option<String>,
        start: f64,
        duration: f64,
        preset: &str,
    ) -> Option<String> {
        let (style, offset_y, font) = match self.text_presets.iter().find(|held| held.id == preset)
        {
            Some(found) => (
                found.style.clone(),
                found.offset_y,
                presets::install_font(&self.host.dirs, found),
            ),
            None => (new_title_style(), None, None),
        };
        // Over the picture, not under it: the first free lane above every
        // occupied one, minted at the top when there is none. A drop onto a
        // lane names its track and is placed there regardless.
        let add = Command::AddTextClip {
            track_id,
            start,
            style: Some(style),
            duration: Some(duration),
            offset_y,
            above: true,
        };
        match font {
            Some((family, path)) => self.apply(Command::Batch {
                commands: vec![Command::AddFont { family, path }, add],
            }),
            None => self.apply(add),
        }
    }

    /// The selected clip's id, when exactly one is selected.
    pub fn sole_selection(&self) -> Option<String> {
        (self.selection.len() == 1).then(|| self.selection[0].clone())
    }

    /// The look being shown over the picture, by id, while one is.
    pub fn audition_of(&self) -> Option<&str> {
        self.audition.as_deref()
    }

    /// Shows a look over the whole picture without laying it down: what a
    /// single click on a Filters card does. The same card clicked again
    /// takes it off; a double-click or the card's plus lays the layer.
    pub fn audition_catalogue(&mut self, id: &str) {
        if self.session.is_none() {
            return;
        }
        let same = self.audition.as_deref() == Some(id);
        if !same && self.audition.is_none() {
            self.notify(
                &t("Showing the look over the picture. Double-click the card, or its plus, to lay it on the timeline"),
                false,
            );
        }
        self.audition = (!same).then(|| id.to_owned());
        self.request_preview();
    }

    /// Lays a filter down as a layer at the playhead - what the Filters
    /// page's double-click and plus do - and ends the showing, the layer
    /// now being on the timeline to see.
    pub fn place_filter_layer(&mut self, id: &str, label: &str) {
        self.audition = None;
        self.place_at_playhead(&format!("filter:{id}:{label}"));
    }

    /// A catalogue filter or effect, applied to the selected clip's chain.
    pub fn apply_catalogue(&mut self, id: &str, video: bool) {
        let Some(clip_id) = self.sole_selection() else {
            self.notify(&t("Select a clip on the timeline first"), true);
            return;
        };
        let Some(clip) = self.clip(&clip_id).cloned() else {
            return;
        };
        if video && !(clip.kind.is_visual() || clip.kind == model::ClipKind::Text) {
            self.notify(
                &t("Select a video, image or text clip on the timeline first"),
                true,
            );
            return;
        }
        if !video && clip.kind == model::ClipKind::Image {
            self.notify("A still has no sound to filter", true);
            return;
        }
        let entry = AppliedFilter::new(id);
        let patch = if video {
            let mut effects = clip.video_effects.clone();
            effects.push(entry);
            ClipPatch {
                video_effects: Some(effects),
                ..ClipPatch::default()
            }
        } else {
            let mut filters = clip.filters.clone();
            filters.push(entry);
            ClipPatch {
                filters: Some(filters),
                ..ClipPatch::default()
            }
        };
        self.apply(Command::UpdateClip { clip_id, patch });
        // Show it: the applied chain is where the knobs are, and a card
        // that did something with no visible result reads as a card that
        // did nothing.
        self.inspector_jump = (
            self.inspector_jump.0 + 1,
            if video { "Effects" } else { "Audio" },
            if video { "Effects" } else { "Sound" },
        );
    }

    /// The clip that ends where `clip` starts, on the same track - the
    /// outgoing side of the cut a transition into `clip` rides. `None` when
    /// nothing is adjacent, which is when a transition has nothing to
    /// dissolve from.
    fn outgoing_of(&self, clip: &Clip) -> Option<&Clip> {
        let frame = self.frame_seconds();
        self.timeline()
            .clips
            .iter()
            .find(|other| {
                other.id != clip.id
                    && other.track_id == clip.track_id
                    && (other.start + other.duration - clip.start).abs() < frame / 2.0
            })
            .map(|other| other.as_ref())
    }

    /// One frame of the active timeline, in seconds - the same tolerance
    /// `concat-export`'s own adjacency test uses, so a clip this call finds
    /// adjacent is one the exporter will too.
    fn frame_seconds(&self) -> f64 {
        let video = &self.timeline().video;
        video.rate_den as f64 / video.rate_num.max(1) as f64
    }

    /// The duration a transition into `clip` can actually run at: the
    /// requested length, clamped to both clips' own durations. `None` when there
    /// is no adjacent clip to dissolve from, or both clips are shorter than a frame -
    /// the ways a transition here would render as nothing at all.
    pub fn transition_duration(&self, clip: &Clip, requested: f64) -> Option<f64> {
        let outgoing = self.outgoing_of(clip)?;
        let frame = self.frame_seconds();
        let max_duration = outgoing.duration.min(clip.duration);
        if max_duration < frame {
            return None;
        }
        let requested = if requested <= 0.0 {
            frame
        } else {
            requested.max(frame)
        };
        let duration = requested.min(max_duration).max(0.0);
        (duration >= frame).then_some(duration)
    }

    pub fn apply_transition(&mut self, id: &str) {
        let Some(clip_id) = self.sole_selection() else {
            self.notify(&t("Select the clip the transition leads into"), true);
            return;
        };
        let Some(clip) = self.clip(&clip_id) else {
            return;
        };
        if !clip.kind.is_visual() {
            self.notify(
                &t("Select a video or image clip on the timeline first"),
                true,
            );
            return;
        }
        let Some(duration) = self.transition_duration(clip, 0.5) else {
            self.notify(
                &t(
                    "There's no room for a transition here - place an adjacent clip \
                     before this one on the same track to dissolve from",
                ),
                true,
            );
            return;
        };
        self.apply(Command::UpdateClip {
            clip_id,
            patch: ClipPatch {
                transition_in: Some(Some(Transition {
                    id: id.to_owned(),
                    duration,
                })),
                ..ClipPatch::default()
            },
        });
    }

    /// Takes the transition off the selected clip, leaving a plain cut.
    pub fn remove_transition(&mut self) {
        let Some(clip_id) = self.sole_selection() else {
            return;
        };
        self.apply(Command::UpdateClip {
            clip_id,
            patch: ClipPatch {
                transition_in: Some(None),
                ..ClipPatch::default()
            },
        });
    }

    /// Sets the selected clip's transition to `seconds`, clamped to what
    /// the cut can actually support - the same ceiling `apply_transition`
    /// enforces, kept here so a drag on the timeline can never ask for more
    /// than the render will give it.
    pub fn set_transition_duration(&mut self, seconds: f64) {
        let Some(clip_id) = self.sole_selection() else {
            return;
        };
        self.set_clip_transition_duration(&clip_id, seconds);
    }

    /// Sets `clip_id`'s transition to `seconds`, clamped to what the cut can
    /// actually support.
    pub fn set_clip_transition_duration(&mut self, clip_id: &str, seconds: f64) {
        let Some(clip) = self.clip(clip_id) else {
            return;
        };
        let Some(existing) = clip.transition_in.clone() else {
            return;
        };
        let Some(duration) = self.transition_duration(clip, seconds.max(0.0)) else {
            return;
        };
        self.apply(Command::UpdateClip {
            clip_id: clip_id.to_owned(),
            patch: ClipPatch {
                transition_in: Some(Some(Transition {
                    duration,
                    ..existing
                })),
                ..ClipPatch::default()
            },
        });
    }

    /// Drops one entry from the selected clip's chains, by the row id the
    /// inspector shows: video effects count from 1, audio filters from 1000.
    pub fn remove_effect(&mut self, row: i32) {
        let Some(clip_id) = self.sole_selection() else {
            return;
        };
        let Some(clip) = self.clip(&clip_id).cloned() else {
            return;
        };
        let patch = if row >= 1000 {
            let index = (row - 1000) as usize;
            let mut filters = clip.filters.clone();
            if index >= filters.len() {
                return;
            }
            filters.remove(index);
            ClipPatch {
                filters: Some(filters),
                ..ClipPatch::default()
            }
        } else {
            let index = (row - 1).max(0) as usize;
            let mut effects = clip.video_effects.clone();
            if index >= effects.len() {
                return;
            }
            effects.remove(index);
            ClipPatch {
                video_effects: Some(effects),
                ..ClipPatch::default()
            }
        };
        self.apply(Command::UpdateClip { clip_id, patch });
    }

    // ── the chains ──
    //
    // The picture's chain and the sound's, edited by index from the
    // inspector's stacks. Adding, removing, reordering and bypassing are
    // each one command; a knob is a stream and goes through the echo, the
    // way every other inspector control does.

    /// One edit to the selected clip's chain, as a command. `edit` returns
    /// false to say it changed nothing.
    fn chain_edit(&mut self, audio: bool, edit: impl FnOnce(&mut Vec<AppliedFilter>) -> bool) {
        let Some(clip_id) = self.sole_selection() else {
            return;
        };
        let Some(clip) = self.clip(&clip_id).cloned() else {
            return;
        };
        let mut chain = if audio {
            clip.filters
        } else {
            clip.video_effects
        };
        if !edit(&mut chain) {
            return;
        }
        let patch = if audio {
            ClipPatch {
                filters: Some(chain),
                ..ClipPatch::default()
            }
        } else {
            ClipPatch {
                video_effects: Some(chain),
                ..ClipPatch::default()
            }
        };
        self.apply(Command::UpdateClip { clip_id, patch });
    }

    pub fn chain_add(&mut self, audio: bool, id: &str) {
        self.apply_catalogue(id, !audio);
    }

    pub fn chain_toggle(&mut self, audio: bool, index: i32) {
        self.chain_edit(audio, |chain| {
            let Some(entry) = usize::try_from(index).ok().and_then(|i| chain.get_mut(i)) else {
                return false;
            };
            entry.enabled = !entry.enabled;
            true
        });
    }

    pub fn chain_move(&mut self, audio: bool, index: i32, delta: i32) {
        self.chain_edit(audio, |chain| {
            let Ok(from) = usize::try_from(index) else {
                return false;
            };
            let Ok(to) = usize::try_from(index + delta) else {
                return false;
            };
            if from >= chain.len() || to >= chain.len() || from == to {
                return false;
            }
            chain.swap(from, to);
            true
        });
    }

    pub fn chain_remove(&mut self, audio: bool, index: i32) {
        self.chain_edit(audio, |chain| {
            let Ok(at) = usize::try_from(index) else {
                return false;
            };
            if at >= chain.len() {
                return false;
            }
            chain.remove(at);
            true
        });
    }

    /// One knob of one link, on the echo. `clip_commit` makes it real.
    pub fn chain_set_param(&mut self, audio: bool, index: i32, key: &str, value: f32) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        self.begin_echo();
        let Some(clip) = self.echo_clip_mut(&id) else {
            return;
        };
        let chain = if audio {
            &mut clip.filters
        } else {
            &mut clip.video_effects
        };
        if let Some(entry) = usize::try_from(index).ok().and_then(|i| chain.get_mut(i)) {
            entry.params.insert(key.to_owned(), f64::from(value));
        }
    }

    /// One knob of the colour panel, on the echo. The adjust package joins
    /// the head of the picture's chain the first time a knob moves, so an
    /// untouched clip carries nothing.
    pub fn adjust_set(&mut self, key: &str, value: f32) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        let point = self.key_point().map(|(_, at)| at);
        self.begin_echo();
        let Some(clip) = self.echo_clip_mut(&id) else {
            return;
        };
        if !clip.kind.is_visual() {
            return;
        }
        let entry = match clip
            .video_effects
            .iter()
            .position(|entry| entry.id == ADJUST_ID)
        {
            Some(entry) => entry,
            None => {
                clip.video_effects.insert(0, AppliedFilter::new(ADJUST_ID));
                0
            }
        };
        let link = &mut clip.video_effects[entry];
        match point {
            // A knob that rides is edited where the playhead is: the value
            // becomes the key there, put on if there was none. Setting the
            // constant under a ride would change nothing on screen.
            Some(at) if link.is_keyed(key) => {
                let ease = ease_before(link.keys_on(key), at);
                link.set_key(key, at, f64::from(value), ease);
            }
            _ => {
                link.params.insert(key.to_owned(), f64::from(value));
            }
        }
    }

    /// The colour link of the selected clip - the one the Adjust panel's
    /// keys go on - by index, with the clip and the playhead's place in it.
    /// None when nothing is selected, it is not a picture, or the playhead
    /// is outside it.
    fn adjust_point(&self) -> Option<(Clip, f64, Option<usize>)> {
        let (clip, at) = self.key_point()?;
        if !clip.kind.is_visual() {
            return None;
        }
        let entry = clip
            .video_effects
            .iter()
            .position(|entry| entry.id == ADJUST_ID);
        Some((clip.clone(), at, entry))
    }

    /// Puts a key on one Adjust knob at the playhead, or takes off the one
    /// there. Like `toggle_key`, the new key holds what the knob is worth
    /// at that instant, so pressing the diamond never moves the picture.
    pub fn toggle_adjust_key(&mut self, key: &str) {
        let Some((clip, at, entry)) = self.adjust_point() else {
            return;
        };
        let default = Catalogue::builtin()
            .get(ADJUST_ID)
            .and_then(|package| package.manifest.params.iter().find(|p| p.key == key))
            .map_or(0.0, |param| param.default);
        let clip_id = clip.id.clone();
        let Some(entry) = entry else {
            // No colour link yet: one goes on, then the key on it, as one
            // edit, so an undo takes both back.
            let mut effects = clip.video_effects.clone();
            effects.insert(0, AppliedFilter::new(ADJUST_ID));
            self.apply(Command::Batch {
                commands: vec![
                    Command::UpdateClip {
                        clip_id: clip_id.clone(),
                        patch: ClipPatch {
                            video_effects: Some(effects),
                            ..ClipPatch::default()
                        },
                    },
                    Command::SetEffectKey {
                        clip_id,
                        entry: 0,
                        key: key.to_owned(),
                        at,
                        value: default,
                        ease: model::KeyEase::LINEAR,
                    },
                ],
            });
            return;
        };
        let link = &clip.video_effects[entry];
        let command = if link.key_at(key, at).is_some() {
            Command::ClearEffectKey {
                clip_id,
                entry,
                key: key.to_owned(),
                at,
            }
        } else {
            Command::SetEffectKey {
                clip_id,
                entry,
                key: key.to_owned(),
                at,
                value: link.value_at(key, at, default),
                ease: ease_before(link.keys_on(key), at),
            }
        };
        self.apply(command);
    }

    /// Takes every key off one Adjust knob, leaving it the value it holds.
    pub fn clear_adjust_keys(&mut self, key: &str) {
        let Some((clip, _, Some(entry))) = self.adjust_point() else {
            return;
        };
        self.apply(Command::ClearEffectKeys {
            clip_id: clip.id,
            entry,
            key: key.to_owned(),
        });
    }

    /// Moves the playhead to an Adjust knob's previous (-1) or next (+1) key.
    pub fn step_adjust_key(&mut self, key: &str, delta: i32) {
        let Some((clip, at, Some(entry))) = self.adjust_point() else {
            return;
        };
        let (prev, next) = clip.video_effects[entry].keys_around(key, at);
        let Some(target) = (if delta < 0 { prev } else { next }) else {
            return;
        };
        self.seek((clip.start + target * clip.duration) as f32);
    }

    // ── gestures ──

    /// A press resolves the selection before anything moves, so a drag that
    /// starts on an already-selected clip carries the whole set.
    pub fn clip_pressed(&mut self, id: &str, additive: bool, edge: i32) {
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        if self.locked(&clip.track_id) {
            return;
        }
        // Whatever the inspector still holds for the clip selected until
        // now lands before the selection moves: a commit flushed after the
        // change would look for it on the newly selected clip and lose it.
        self.flush_commit();
        let already = self.selection.iter().any(|held| held == id);
        self.selection = if additive {
            if already {
                self.selection
                    .iter()
                    .filter(|held| held.as_str() != id)
                    .cloned()
                    .collect()
            } else {
                let mut next = self.selection.clone();
                next.push(id.to_owned());
                next
            }
        } else if already {
            self.selection.clone()
        } else {
            vec![id.to_owned()]
        };

        self.begin_echo();
        if edge == 2 && self.selection.len() <= 1 {
            if let Some(transition) = clip.transition_in.as_ref() {
                self.gesture = Gesture::TransitionResize {
                    clip: id.to_owned(),
                    original_duration: transition.duration as f32,
                };
                return;
            }
        }
        if edge >= 0 && self.selection.len() <= 1 {
            self.gesture = Gesture::Trim {
                clip: id.to_owned(),
                edge: if edge == 0 { Edge::Start } else { Edge::End },
                start: clip.start as f32,
                duration: clip.duration as f32,
                source_start: clip.source_start as f32,
            };
            return;
        }
        let mut moving = if self.selection.iter().any(|held| held == id) {
            self.selection.clone()
        } else {
            vec![id.to_owned()]
        };
        // A detached sound travels with its picture and the picture with
        // its sound: the pair stays in step unless one of them is moved
        // on its own lane by a lock (#105).
        let partners: Vec<String> = moving
            .iter()
            .filter_map(|clip_id| self.clip(clip_id))
            .flat_map(|clip| {
                let mut found: Vec<String> = clip.detached_from.iter().cloned().collect();
                found.extend(
                    self.timeline()
                        .clips
                        .iter()
                        .filter(|other| other.detached_from.as_deref() == Some(clip.id.as_str()))
                        .map(|other| other.id.clone()),
                );
                found
            })
            .filter(|partner| !moving.contains(partner))
            .collect();
        moving.extend(partners);
        let origins = moving
            .iter()
            .filter_map(|clip_id| {
                let clip = self.clip(clip_id)?;
                Some(MoveOrigin {
                    clip: clip.id.clone(),
                    start: clip.start as f32,
                    row: self.row_of(&clip.track_id),
                })
            })
            .collect();
        self.gesture = Gesture::Move {
            primary: id.to_owned(),
            origins,
            lanes: self.lane_heights(),
        };
    }

    /// The pointer moved: the echo follows it.
    pub fn clip_dragged(&mut self, seconds: f32, pixels: f32) {
        let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        match &gesture {
            Gesture::Move {
                primary,
                origins,
                lanes,
            } => {
                let Some(anchor) = origins.iter().find(|origin| &origin.clip == primary) else {
                    self.gesture = gesture;
                    return;
                };
                let threshold = 8.0 * self.lanes.seconds_per_pixel;
                let snapped = self.snapped(anchor.start + seconds, threshold, primary);
                let shift = snapped - anchor.start;
                let rows = nearest_row(lanes, row_top(lanes, anchor.row) + pixels) - anchor.row;
                let count = self.timeline().tracks.len() as i32;
                let moves: Vec<(String, f32, Option<String>)> = origins
                    .iter()
                    .map(|origin| {
                        let row = (origin.row + rows).clamp(0, count - 1);
                        let onto = self
                            .row_track(row)
                            .filter(|track| !self.locked(&track.id))
                            .map(|track| track.id.clone());
                        (origin.clip.clone(), (origin.start + shift).max(0.0), onto)
                    })
                    .collect();
                for (id, start, track) in moves {
                    if let Some(clip) = self.echo_clip_mut(&id) {
                        clip.start = f64::from(start);
                        if let Some(track) = track {
                            clip.track_id = track;
                        }
                    }
                }
            }
            Gesture::Trim {
                clip,
                edge,
                start,
                duration,
                source_start,
            } => {
                let (id, edge) = (clip.clone(), *edge);
                let (start, duration, source_start) = (*start, *duration, *source_start);
                let threshold = 8.0 * self.lanes.seconds_per_pixel;
                let speed = self.clip(&id).map_or(1.0, |clip| clip.speed as f32);
                if edge == Edge::Start {
                    // The head cannot pass the tail, and cannot pull material
                    // out of a file that has none before the in-point.
                    let wanted = self.snapped(start + seconds, threshold, &id);
                    let limit = start + duration - MIN_DURATION;
                    let at = wanted.clamp((start - source_start / speed.max(0.01)).max(0.0), limit);
                    let delta = at - start;
                    if let Some(clip) = self.echo_clip_mut(&id) {
                        clip.start = f64::from(at);
                        clip.duration = f64::from(duration - delta);
                        clip.source_start = f64::from(source_start + delta * speed);
                    }
                } else {
                    let wanted = self.snapped(start + duration + seconds, threshold, &id);
                    let at = wanted.max(start + MIN_DURATION);
                    if let Some(clip) = self.echo_clip_mut(&id) {
                        clip.duration = f64::from(at - start);
                    }
                }
            }
            Gesture::TransitionResize {
                clip,
                original_duration,
            } => {
                let id = clip.clone();
                let original_duration = *original_duration;
                if let Some(clip_ref) = self.clip(&id).cloned() {
                    let wanted = (original_duration + seconds).max(0.0) as f64;
                    if let Some(duration) = self.transition_duration(&clip_ref, wanted) {
                        if let Some(echo_clip) = self.echo_clip_mut(&id) {
                            if let Some(t) = echo_clip.transition_in.as_mut() {
                                t.duration = duration;
                            }
                        }
                    }
                }
            }
            // A stage gesture is the monitor's; the lanes have nothing to
            // add to it.
            Gesture::None
            | Gesture::StageMove { .. }
            | Gesture::StageScale { .. }
            | Gesture::StageRotate { .. }
            | Gesture::StageStretch { .. }
            | Gesture::TextWidth { .. }
            | Gesture::TextHeight { .. }
            | Gesture::Paint { .. } => {}
        }
        self.gesture = gesture;
    }

    /// The pointer let go: the whole gesture becomes one command.
    pub fn clip_released(&mut self) {
        let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        let Some(echo) = self.echo.as_ref() else {
            return;
        };
        match gesture {
            Gesture::Move { origins, .. } => {
                let moves: Vec<ClipMove> = origins
                    .iter()
                    .filter_map(|origin| {
                        let after = echo.active().clip(&origin.clip)?;
                        Some(ClipMove {
                            clip_id: origin.clip.clone(),
                            start: after.start,
                            track_id: after.track_id.clone(),
                        })
                    })
                    .collect();
                self.echo = None;
                if !moves.is_empty() {
                    self.apply(Command::MoveClips { moves });
                }
            }
            Gesture::Trim {
                clip,
                edge,
                start,
                duration,
                ..
            } => {
                let after = echo.active().clip(&clip).cloned();
                self.echo = None;
                let Some(after) = after else { return };
                let delta = match edge {
                    Edge::Start => after.start - f64::from(start),
                    Edge::End => after.duration - f64::from(duration),
                };
                if delta.abs() > 1e-6 {
                    self.apply(Command::TrimClip {
                        clip_id: clip,
                        edge: match edge {
                            Edge::Start => TrimEdge::Start,
                            Edge::End => TrimEdge::End,
                        },
                        delta,
                        // Magnetic: the lane closes behind the edge.
                        // https://github.com/jub0t/Concat/issues/106
                        ripple: self.prefs.magnetic,
                    });
                }
            }
            Gesture::TransitionResize {
                clip,
                original_duration,
            } => {
                let after = echo.active().clip(&clip).cloned();
                self.echo = None;
                let Some(after) = after else { return };
                let new_duration = after
                    .transition_in
                    .as_ref()
                    .map(|t| t.duration)
                    .unwrap_or(0.0);
                if (new_duration - f64::from(original_duration)).abs() > 1e-4 {
                    self.set_clip_transition_duration(&clip, new_duration);
                }
            }
            Gesture::None => {
                self.echo = None;
            }
            // Not a lane gesture: hand it back untouched, echo and all.
            other @ (Gesture::StageMove { .. }
            | Gesture::StageScale { .. }
            | Gesture::StageRotate { .. }
            | Gesture::StageStretch { .. }
            | Gesture::TextWidth { .. }
            | Gesture::TextHeight { .. }
            | Gesture::Paint { .. }) => {
                self.gesture = other;
            }
        }
    }

    // ── the inspector ──

    /// One field of the selected clip, on the echo. `clip_commit` turns the
    /// accumulated edits into commands.
    pub fn clip_set(&mut self, field: ClipField, value: f32) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        self.commit_target = Some(id.clone());
        // The media's tracks, read before the echo is borrowed: a row of
        // the Audio panel's list is a stream index of the file.
        let audio_tracks: Vec<u32> = if field == ClipField::AudioTrack {
            self.clip(&id)
                .and_then(|clip| self.project().media_by_id(&clip.media_id))
                .map(|item| item.audio_tracks.iter().map(|track| track.index).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        // Where the playhead sits in the clip: a keyed property is written
        // there, as a key.
        let at = self
            .clip(&id)
            .map(|clip| place_in(clip, self.playhead))
            .unwrap_or(0.0);
        self.begin_echo();
        let value = f64::from(value);
        let Some(clip) = self.echo_clip_mut(&id) else {
            return;
        };
        let text = clip.text.get_or_insert_with(TextStyle::default);
        match field {
            ClipField::Scale => {
                write_keyable(clip, model::KeyProperty::Scale, value.clamp(0.05, 8.0), at)
            }
            ClipField::AudioTrack => {
                // The first row is the file's default and is stored as such,
                // so a clip on the first track saves as every clip did before
                // there were tracks to choose.
                let row = value.max(0.0) as usize;
                clip.audio_stream = (row > 0).then(|| audio_tracks.get(row).copied()).flatten();
            }
            ClipField::StretchX => clip.stretch_x = value.clamp(0.1, 10.0),
            ClipField::StretchY => clip.stretch_y = value.clamp(0.1, 10.0),
            ClipField::OffsetX => write_keyable(
                clip,
                model::KeyProperty::OffsetX,
                value.clamp(-1.0, 1.0),
                at,
            ),
            ClipField::OffsetY => write_keyable(
                clip,
                model::KeyProperty::OffsetY,
                value.clamp(-1.0, 1.0),
                at,
            ),
            ClipField::Rotation => write_keyable(
                clip,
                model::KeyProperty::Rotation,
                value.clamp(-180.0, 180.0),
                at,
            ),
            ClipField::Opacity => {
                write_keyable(clip, model::KeyProperty::Opacity, value.clamp(0.0, 1.0), at)
            }
            ClipField::CutoutFeather => {
                clip.cutout.get_or_insert_with(model::Cutout::auto).feather =
                    value.clamp(0.0, model::MAX_FEATHER);
            }
            ClipField::Volume => {
                write_keyable(clip, model::KeyProperty::Volume, value.max(0.0), at)
            }
            ClipField::Speed => {
                let speed = value.clamp(0.0625, 16.0);
                clip.duration = (clip.duration * clip.speed / speed).max(f64::from(MIN_DURATION));
                clip.speed = speed;
            }
            ClipField::PreservePitch => clip.preserve_pitch = value != 0.0,
            ClipField::Reverse => clip.reverse = value != 0.0,
            ClipField::FlipH => clip.flip_h = value != 0.0,
            ClipField::FlipV => clip.flip_v = value != 0.0,
            ClipField::Blend => {
                let mode = concat_core::Blend::ALL
                    .get(value.max(0.0) as usize)
                    .copied()
                    .unwrap_or_default();
                clip.blend = if mode == concat_core::Blend::Normal {
                    String::new()
                } else {
                    mode.name().to_owned()
                };
            }
            ClipField::CropLeft
            | ClipField::CropTop
            | ClipField::CropRight
            | ClipField::CropBottom => {
                let mut crop = clip.crop.unwrap_or_default();
                let edge = match field {
                    ClipField::CropLeft => &mut crop.left,
                    ClipField::CropTop => &mut crop.top,
                    ClipField::CropRight => &mut crop.right,
                    _ => &mut crop.bottom,
                };
                *edge = value.clamp(0.0, 0.9);
                let crop = crop.tidy();
                clip.crop = (!crop.is_none()).then_some(crop);
            }
            ClipField::SpeedCurve => {
                // The same arithmetic as the command: the source covered is
                // held, so the length follows the curve's mean.
                let curve = if value < 0.0 {
                    None
                } else {
                    concat_project::speed::preset(value as usize)
                };
                let covered = clip.duration * clip.speed;
                let mean = curve
                    .as_ref()
                    .map(|points| concat_project::speed::mean_of(points))
                    .unwrap_or(clip.speed)
                    .clamp(0.0625, 16.0);
                clip.speed_curve = curve;
                clip.speed = mean;
                clip.duration = (covered / mean).max(f64::from(MIN_DURATION));
            }
            ClipField::FadeIn => clip.fade_in = value.clamp(0.0, clip.duration / 2.0),
            ClipField::FadeOut => clip.fade_out = value.clamp(0.0, clip.duration / 2.0),
            ClipField::FontSize => text.font_size = value.clamp(0.01, 0.5),
            ClipField::FontWeight => text.font_weight = value.clamp(100.0, 900.0),
            ClipField::Italic => text.italic = value != 0.0,
            ClipField::TextOpacity => text.opacity = value.clamp(0.0, 1.0),
            ClipField::Align => {
                text.align = match value as i32 {
                    0 => TextAlign::Left,
                    2 => TextAlign::Right,
                    _ => TextAlign::Center,
                }
            }
            ClipField::StrokeWidth => text.stroke_width = value.clamp(0.0, 0.15),
            ClipField::Shadow => text.shadow = value != 0.0,
            ClipField::LineHeight => text.line_height = value.clamp(0.7, 2.5),
            ClipField::Tracking => text.tracking = value.clamp(-0.05, 0.3),
            ClipField::TextWidth => text.max_width = value.clamp(0.0, 2.0),
            ClipField::TextHeight => text.max_height = value.clamp(0.0, 2.0),
            // The stroke's opacity is its colour's alpha; see `hex_rgba`.
            ClipField::StrokeOpacity => {
                let edge = colour_of(&text.stroke_color);
                text.stroke_color = hex_rgba(slint::Color::from_argb_u8(
                    (value.clamp(0.0, 1.0) * 255.0).round() as u8,
                    edge.red(),
                    edge.green(),
                    edge.blue(),
                ));
            }
        }
        // A media clip has no text; the placeholder must not linger.
        if clip.kind != model::ClipKind::Text {
            clip.text = None;
        }
    }

    pub fn clip_set_text(&mut self, field: ClipTextField, value: &str) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        self.commit_target = Some(id.clone());
        self.begin_echo();
        let Some(clip) = self.echo_clip_mut(&id) else {
            return;
        };
        if clip.kind != model::ClipKind::Text {
            return;
        }
        let text = clip.text.get_or_insert_with(TextStyle::default);
        match field {
            ClipTextField::Content => {
                text.content = value.to_owned();
                let first = value.lines().next().unwrap_or("").trim().to_owned();
                clip.name = if first.is_empty() {
                    "Title".into()
                } else {
                    first
                };
            }
            ClipTextField::FontFamily => text.font_family = value.to_owned(),
            _ => {}
        }
        // The words are on the echo now; show them. A title being typed is
        // painted in memory at the monitor's size, the way a grip drag is,
        // so the picture keeps up with the keystrokes while the commit
        // still lands once, on the way out of the field.
        //
        // Pending from the first keystroke, not from the field's blur: the
        // echo is dropped by anything that changes the edit - a press on a
        // clip, an undo, a command from the menu - and `flush_commit` only
        // lands what is pending. Words typed and not yet blurred were on
        // the echo alone, and went with it. Nothing starts the timer here;
        // the blur does, and a flush before then lands them too.
        self.commit_pending = true;
        self.request_preview();
    }

    pub fn clip_set_colour(&mut self, field: ClipTextField, value: slint::Color) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        self.commit_target = Some(id.clone());
        self.begin_echo();
        let Some(clip) = self.echo_clip_mut(&id) else {
            return;
        };
        if clip.kind != model::ClipKind::Text {
            return;
        }
        let text = clip.text.get_or_insert_with(TextStyle::default);
        match field {
            ClipTextField::Color => text.color = hex_of(value),
            ClipTextField::StrokeColor => text.stroke_color = hex_rgba(value),
            ClipTextField::Background => text.background = hex_with_alpha(value),
            _ => {}
        }
    }

    /// The inspector's gesture is over: what differs between the echo and
    /// the session becomes commands, as one batch.
    /// An inspector control changed something on the echo and wants it
    /// committed. Held, not applied: a knob commits on every move, and the
    /// command, the mix and the publish happen once the moves pause. The
    /// monitor follows the echo in the meantime.
    pub fn clip_commit(&mut self) {
        self.commit_pending = true;
        self.request_preview();
        self.commit_timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_millis(120),
            || {
                crate::host::Shell::with(|shell, app| {
                    shell.studio.borrow_mut().flush_commit();
                    shell.studio.borrow().publish(&app, &shell.models);
                });
            },
        );
    }

    /// Lands the held commit now, if there is one. Called before anything
    /// that would drop the echo it lives on - a command from elsewhere, an
    /// undo, a gesture on the lanes or the stage.
    pub fn flush_commit(&mut self) {
        if !self.commit_pending {
            return;
        }
        self.commit_pending = false;
        self.commit_timer.stop();
        self.commit_now();
    }

    fn commit_now(&mut self) {
        // The clip the echo was written for, whatever is selected now; see
        // `commit_target`. The selection is the fallback for a commit asked
        // for with nothing written, which has nothing to land anyway.
        let Some(id) = self.commit_target.take().or_else(|| self.sole_selection()) else {
            self.echo = None;
            return;
        };
        let (Some(after), Some(before)) = (
            self.echo
                .as_ref()
                .and_then(|echo| echo.active().clip(&id))
                .cloned(),
            self.session
                .as_ref()
                .and_then(|session| session.project().active().clip(&id))
                .cloned(),
        ) else {
            self.echo = None;
            return;
        };
        let mut commands = Vec::new();
        if after.scale != before.scale
            || after.offset_x != before.offset_x
            || after.offset_y != before.offset_y
            || after.rotation != before.rotation
            || after.stretch_x != before.stretch_x
            || after.stretch_y != before.stretch_y
        {
            commands.push(Command::SetClipTransform {
                clip_id: id.clone(),
                scale: Some(after.scale),
                offset_x: Some(after.offset_x),
                offset_y: Some(after.offset_y),
                rotation: Some(after.rotation),
                stretch_x: Some(after.stretch_x),
                stretch_y: Some(after.stretch_y),
            });
        }
        if after.cutout != before.cutout {
            commands.push(Command::SetClipCutout {
                clip_id: id.clone(),
                cutout: after.cutout.clone(),
            });
        }
        if after.speed_curve != before.speed_curve {
            commands.push(Command::SetClipSpeedCurve {
                clip_id: id.clone(),
                curve: after.speed_curve.clone(),
            });
        } else if after.speed != before.speed {
            commands.push(Command::SetClipSpeed {
                clip_id: id.clone(),
                speed: after.speed,
            });
        }
        let mut patch = ClipPatch::default();
        if after.name != before.name {
            patch.name = Some(after.name.clone());
        }
        if after.volume != before.volume {
            patch.volume = Some(after.volume);
        }
        if after.fade_in != before.fade_in {
            patch.fade_in = Some(after.fade_in);
        }
        if after.fade_out != before.fade_out {
            patch.fade_out = Some(after.fade_out);
        }
        if after.opacity != before.opacity {
            patch.opacity = Some(after.opacity);
        }
        if after.preserve_pitch != before.preserve_pitch {
            patch.preserve_pitch = Some(after.preserve_pitch);
        }
        if after.audio_stream != before.audio_stream {
            patch.audio_stream = Some(after.audio_stream);
        }
        if after.reverse != before.reverse {
            patch.reverse = Some(after.reverse);
        }
        if after.flip_h != before.flip_h {
            patch.flip_h = Some(after.flip_h);
        }
        if after.flip_v != before.flip_v {
            patch.flip_v = Some(after.flip_v);
        }
        if after.blend != before.blend {
            patch.blend = Some(if after.blend.is_empty() {
                "normal".to_owned()
            } else {
                after.blend.clone()
            });
        }
        if after.crop != before.crop {
            patch.crop = Some(after.crop);
        }
        commands.extend(key_commands(&id, &before, &after));
        if after.text != before.text {
            patch.text = Some(after.text.clone());
        }
        if after.video_effects != before.video_effects {
            patch.video_effects = Some(after.video_effects.clone());
        }
        if after.filters != before.filters {
            patch.filters = Some(after.filters.clone());
        }
        if patch != ClipPatch::default() {
            commands.push(Command::UpdateClip { clip_id: id, patch });
        }
        self.echo = None;
        if commands.is_empty() {
            return;
        }
        // One undo step per gesture, not per pointer move: a commit that
        // changes the same things on the same clip as the last, within a
        // moment of it, folds into the last one's step. The editor does the
        // folding; this decides when a pause is long enough to be a new
        // gesture on the same knob.
        let key = format!("{}:{}", after.id, commit_key(&commands));
        let now = std::time::Instant::now();
        let continues = self
            .last_commit
            .as_ref()
            .is_some_and(|(last, at)| *last == key && now.duration_since(*at).as_millis() < 900);
        if !continues && let Some(session) = self.session.as_mut() {
            session.end_gesture();
        }
        let command = match commands.len() {
            1 => commands.remove(0),
            _ => Command::Batch { commands },
        };
        self.apply_within(&key, command);
        self.last_commit = Some((key, now));
    }

    // ── the stage ──
    //
    // The monitor as a place to edit, not only to look: the pictures under
    // the playhead can be grabbed, slid, scaled and turned where they are
    // drawn. The same echo-then-command shape as the lanes and the
    // inspector, with one difference: the echo is composited too, so the
    // frame follows the pointer and not only the outline does.

    /// Where `clip` lands in the output frame. The compositor's own rule —
    /// contain-fitted, then the transform — restated in fractions.
    pub fn footprint(&self, clip: &Clip) -> Footprint {
        let (width, height) = self.output_size();
        let (width, height) = (f64::from(width.max(1)), f64::from(height.max(1)));
        let painted = (clip.kind == model::ClipKind::Text)
            .then(|| self.title_blocks.get(&clip.id).copied())
            .flatten();
        let (w, h) = if let Some(((bw, bh), _)) = painted {
            // The painter said how big the block came out.
            (
                f64::from(bw) * clip.scale / width,
                f64::from(bh) * clip.scale / height,
            )
        } else if clip.kind == model::ClipKind::Text {
            // Not painted yet: a reading of the style until it is. An em is
            // `font_size` of the frame's height, a glyph runs about six
            // tenths of one, and a little air is left around the block the
            // way the plate would.
            let text = clip.text.clone().unwrap_or_default();
            let lines: Vec<&str> = text.content.lines().collect();
            let rows = lines.len().max(1) as f64;
            let longest = lines
                .iter()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0)
                .max(1) as f64;
            let em = text.font_size.max(0.005) * height;
            let glyph = 0.6 * em + text.tracking * height;
            (
                (longest * glyph + 0.6 * em) * clip.scale / width,
                (rows * em * text.line_height.max(0.5) + 0.5 * em) * clip.scale / height,
            )
        } else {
            let media = self.project().media_by_id(&clip.media_id);
            let source = media
                .and_then(|item| Some((item.width?, item.height?)))
                .filter(|(w, h)| *w > 0 && *h > 0)
                .map(|(w, h)| (f64::from(w), f64::from(h)))
                // What is left after the crop is what gets fitted.
                .map(|(w, h)| match clip.crop {
                    Some(crop) => (
                        w * (1.0 - crop.left - crop.right).max(0.1),
                        h * (1.0 - crop.top - crop.bottom).max(0.1),
                    ),
                    None => (w, h),
                });
            match source {
                Some((sw, sh)) => {
                    let fit = (width / sw).min(height / sh);
                    (
                        sw * fit * clip.scale / width,
                        sh * fit * clip.scale / height,
                    )
                }
                // Dimensions never learnt: the frame's own shape, which is
                // what the compositor falls back to as well.
                None => (clip.scale, clip.scale),
            }
        };
        // Then pulled along each axis, as the compositor pulls it - a
        // picture, that is. A title's box is its style's, and the compositor
        // never stretches one; see titles.rs.
        // https://github.com/jub0t/Concat/issues/119
        let (w, h) = if clip.kind == model::ClipKind::Text {
            (w, h)
        } else {
            (w * clip.stretch_x, h * clip.stretch_y)
        };
        // The placement at the playhead: the clip's own, or its ride's
        // where a property is keyed, so the box follows the keys.
        let at = place_in(clip, self.playhead);
        let base = concat_core::timeline::Transform {
            scale: shown(clip, model::KeyProperty::Scale, at),
            offset_x: shown(clip, model::KeyProperty::OffsetX, at),
            offset_y: shown(clip, model::KeyProperty::OffsetY, at),
            rotation: shown(clip, model::KeyProperty::Rotation, at),
            stretch_x: clip.stretch_x,
            stretch_y: clip.stretch_y,
        };
        let placed = base;
        let factor = placed.scale / clip.scale.max(1e-6);
        // A title anchored on an edge paints its block beside the clip's
        // position, not on it; the box goes where the block is. The offset
        // is canvas pixels, so it scales, stretches and turns with the clip
        // before it becomes a fraction of the frame.
        let (ax, ay) = match painted {
            Some((_, (dx, dy))) if dx != 0 || dy != 0 => {
                let px = f64::from(dx) * placed.scale * clip.stretch_x;
                let py = f64::from(dy) * placed.scale * clip.stretch_y;
                let (sin, cos) = placed.rotation.to_radians().sin_cos();
                (
                    (px * cos - py * sin) / width,
                    (px * sin + py * cos) / height,
                )
            }
            _ => (0.0, 0.0),
        };
        Footprint {
            cx: 0.5 + placed.offset_x + ax,
            cy: 0.5 + placed.offset_y + ay,
            w: w * factor,
            h: h * factor,
            rotation: placed.rotation,
        }
    }

    /// The pictures under the playhead on lanes that are showing, bottom of
    /// the stack first — the order the compositor lays them down, and the
    /// order the overlay draws them in.
    fn stage_clips(&self) -> Vec<&Clip> {
        let timeline = self.timeline();
        let playhead = f64::from(self.playhead);
        let showing: HashSet<&str> = timeline
            .tracks
            .iter()
            .filter(|track| track.visible)
            .map(|track| track.id.as_str())
            .collect();
        let mut clips: Vec<&Clip> = timeline
            .clips
            .iter()
            .map(|clip| clip.as_ref())
            .filter(|clip| {
                (clip.kind.is_visual() || clip.kind == model::ClipKind::Text)
                    && showing.contains(clip.track_id.as_str())
                    && clip.start <= playhead
                    && playhead < clip.start + clip.duration
            })
            .collect();
        // Rows count from the top; the bottom of the stack is the highest.
        clips.sort_by_key(|clip| std::cmp::Reverse(self.row_of(&clip.track_id)));
        clips
    }

    pub fn stage_items(&self) -> Vec<StageItemData> {
        self.stage_clips()
            .into_iter()
            .map(|clip| {
                let footprint = self.footprint(clip);
                StageItemData {
                    id: clip.id.as_str().into(),
                    kind: kind_of(clip),
                    selected: self.selection.iter().any(|id| id == &clip.id),
                    cx: footprint.cx as f32,
                    cy: footprint.cy as f32,
                    w: footprint.w as f32,
                    h: footprint.h as f32,
                    rotation: footprint.rotation as f32,
                    scale: clip.scale as f32,
                }
            })
            .collect()
    }

    /// The topmost picture under a frame point, skipping locked lanes the
    /// way a press on the lanes does.
    fn stage_hit(&self, x: f64, y: f64) -> Option<String> {
        let frame = self.output_size();
        self.stage_clips()
            .into_iter()
            .rev()
            .filter(|clip| !self.locked(&clip.track_id))
            .find(|clip| self.footprint(clip).contains(x, y, frame))
            .map(|clip| clip.id.clone())
    }

    /// A press on the stage floor: resolve the selection, then arm a move
    /// of everything selected that is under the playhead. Empty stage
    /// clears the selection, as an empty lane does.
    pub fn stage_pressed(&mut self, x: f32, y: f32, additive: bool) {
        if let Some(clip) = self.paint_target() {
            let point = self.stage_to_source(&clip, f64::from(x), f64::from(y));
            self.gesture = Gesture::Paint {
                clip: clip.id.clone(),
                tool: BRUSHES[self.brush.min(BRUSHES.len() - 1)],
                size: self.brush_size,
                at: self.source_at_playhead(&clip),
                points: vec![point],
                screen: vec![(x, y)],
            };
            return;
        }
        let (x, y) = (f64::from(x), f64::from(y));
        // As on the lanes: what the inspector holds for the current
        // selection lands before the selection changes under it - and a
        // press on the floor, which clears it, is the commonest way out of
        // a title's text field.
        self.flush_commit();
        let Some(id) = self.stage_hit(x, y) else {
            if !additive {
                self.selection.clear();
            }
            self.gesture = Gesture::None;
            return;
        };
        let already = self.selection.iter().any(|held| held == &id);
        if additive {
            if already {
                self.selection.retain(|held| held != &id);
            } else {
                self.selection.push(id.clone());
            }
        } else if !already {
            self.selection = vec![id.clone()];
        }
        if !self.selection.iter().any(|held| held == &id) {
            self.gesture = Gesture::None;
            return;
        }
        let origins: Vec<StageOrigin> = self
            .stage_clips()
            .into_iter()
            .filter(|clip| {
                self.selection.iter().any(|held| held == &clip.id) && !self.locked(&clip.track_id)
            })
            .map(|clip| {
                // A keyed clip is dragged from where its ride has it at
                // the playhead, not from a constant nothing shows.
                let at = place_in(clip, self.playhead);
                StageOrigin {
                    clip: clip.id.clone(),
                    offset_x: shown(clip, model::KeyProperty::OffsetX, at),
                    offset_y: shown(clip, model::KeyProperty::OffsetY, at),
                }
            })
            .collect();
        self.begin_echo();
        self.stage_guides.clear();
        self.gesture = Gesture::StageMove {
            primary: id,
            origins,
            from: (x, y),
        };
    }

    /// Where a picture's bounds pull to on one axis. Every candidate is a
    /// feature of the moving picture — an edge or the centre — against a
    /// target — the frame's edges and centre, and every other picture's —
    /// and the nearest pair inside `pull` wins. Returns the shift that lands
    /// it and the target's position, for the guide.
    fn stage_snap(features: [f64; 3], targets: &[f64], pull: f64) -> Option<(f64, f64)> {
        let mut best: Option<(f64, f64)> = None;
        for feature in features {
            for &target in targets {
                let shift = target - feature;
                if shift.abs() < pull && best.is_none_or(|(held, _)| shift.abs() < held.abs()) {
                    best = Some((shift, target));
                }
            }
        }
        best
    }

    /// A press on a grip of `id`'s box: a corner scales, the handle above
    /// turns. Both work about the picture's centre, in frame pixels.
    pub fn stage_grip_pressed(&mut self, id: &str, grip: i32, x: f32, y: f32) {
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        if self.locked(&clip.track_id) {
            return;
        }
        let footprint = self.footprint(&clip);
        let (width, height) = self.output_size();
        let centre = (
            footprint.cx * f64::from(width),
            footprint.cy * f64::from(height),
        );
        let dx = f64::from(x) * f64::from(width) - centre.0;
        let dy = f64::from(y) * f64::from(height) - centre.1;
        let half = footprint.half_bounds((width, height));
        self.flush_commit();
        self.begin_echo();
        self.gesture = if grip == 4 {
            Gesture::StageRotate {
                clip: id.to_owned(),
                rotation: shown(
                    &clip,
                    model::KeyProperty::Rotation,
                    place_in(&clip, self.playhead),
                ),
                centre,
                from: dy.atan2(dx),
            }
        } else if (grip == 6 || grip == 8) && clip.kind == model::ClipKind::Text {
            // A title's side grips set where its words wrap, not how wide
            // its glyphs are; see `Gesture::TextWidth`.
            Gesture::TextWidth {
                clip: id.to_owned(),
                centre,
                rotation: clip.rotation,
            }
        } else if (grip == 5 || grip == 7) && clip.kind == model::ClipKind::Text {
            // And the top and bottom grips size its box, never its glyphs.
            // https://github.com/jub0t/Concat/issues/119
            Gesture::TextHeight {
                clip: id.to_owned(),
                centre,
                rotation: clip.rotation,
            }
        } else if grip >= 5 {
            // 5 top, 6 right, 7 bottom, 8 left: the pointer's offset from
            // the centre, turned back into the box's own frame.
            let across = grip == 6 || grip == 8;
            let (sin, cos) = clip.rotation.to_radians().sin_cos();
            let along = if across {
                dx * cos + dy * sin
            } else {
                -dx * sin + dy * cos
            };
            Gesture::StageStretch {
                clip: id.to_owned(),
                across,
                stretch: if across {
                    clip.stretch_x
                } else {
                    clip.stretch_y
                },
                centre,
                from: along.abs().max(1.0),
                rotation: clip.rotation,
            }
        } else {
            Gesture::StageScale {
                clip: id.to_owned(),
                scale: shown(
                    &clip,
                    model::KeyProperty::Scale,
                    place_in(&clip, self.playhead),
                ),
                centre,
                from: dx.hypot(dy).max(1.0),
                half,
            }
        };
    }

    /// The pointer moved with a stage gesture live: the echo follows, and
    /// the monitor composites it.
    pub fn stage_dragged(&mut self, x: f32, y: f32, snap: bool) {
        let mut gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        if let Gesture::Paint {
            clip,
            points,
            screen,
            ..
        } = &mut gesture
        {
            if let Some(clip) = self.clip(clip).cloned() {
                points.push(self.stage_to_source(&clip, f64::from(x), f64::from(y)));
                screen.push((x, y));
            }
            self.gesture = gesture;
            return;
        }
        let (x, y) = (f64::from(x), f64::from(y));
        let (width, height) = self.output_size();
        match &gesture {
            Gesture::StageMove {
                primary,
                origins,
                from,
            } => {
                let (mut dx, mut dy) = (x - from.0, y - from.1);
                if snap {
                    // Shift holds the axis the drag is mostly along, measured
                    // in pixels so a tall frame does not bias it.
                    if (dx * f64::from(width)).abs() >= (dy * f64::from(height)).abs() {
                        dy = 0.0;
                    } else {
                        dx = 0.0;
                    }
                }
                self.stage_guides.clear();
                if self.lanes.snap {
                    // The same pull on both axes, in frame pixels: a
                    // hundredth of the long side, which is about eight
                    // pixels on a stage of the size a laptop gives it.
                    let pull = 0.01 * f64::from(width.max(height));
                    let frame = (width, height);
                    let moving = origins
                        .iter()
                        .find(|origin| &origin.clip == primary)
                        .and_then(|origin| self.clip(&origin.clip).map(|clip| (origin, clip)));
                    if let Some((origin, clip)) = moving {
                        let (hw, hh) = self.footprint(clip).half_bounds(frame);
                        let cx = 0.5 + origin.offset_x + dx;
                        let cy = 0.5 + origin.offset_y + dy;
                        // The frame's own lines, then every picture that is
                        // staying put.
                        let mut xs = vec![0.0, 0.5, 1.0];
                        let mut ys = vec![0.0, 0.5, 1.0];
                        for other in self.stage_clips() {
                            if origins.iter().any(|origin| origin.clip == other.id) {
                                continue;
                            }
                            let footprint = self.footprint(other);
                            let (ow, oh) = footprint.half_bounds(frame);
                            xs.extend([footprint.cx - ow, footprint.cx, footprint.cx + ow]);
                            ys.extend([footprint.cy - oh, footprint.cy, footprint.cy + oh]);
                        }
                        if let Some((shift, at)) =
                            Self::stage_snap([cx - hw, cx, cx + hw], &xs, pull / f64::from(width))
                        {
                            dx += shift;
                            self.stage_guides.push(StageGuideData {
                                vertical: true,
                                at: at as f32,
                            });
                        }
                        if let Some((shift, at)) =
                            Self::stage_snap([cy - hh, cy, cy + hh], &ys, pull / f64::from(height))
                        {
                            dy += shift;
                            self.stage_guides.push(StageGuideData {
                                vertical: false,
                                at: at as f32,
                            });
                        }
                    }
                }
                let playhead = self.playhead;
                for origin in origins {
                    if let Some(clip) = self.echo_clip_mut(&origin.clip) {
                        let at = place_in(clip, playhead);
                        let (x, y) = (
                            (origin.offset_x + dx).clamp(-1.0, 1.0),
                            (origin.offset_y + dy).clamp(-1.0, 1.0),
                        );
                        write_keyable(clip, model::KeyProperty::OffsetX, x, at);
                        write_keyable(clip, model::KeyProperty::OffsetY, y, at);
                    }
                }
            }
            Gesture::StageScale {
                clip,
                scale,
                centre,
                from,
                half,
            } => {
                let dx = x * f64::from(width) - centre.0;
                let dy = y * f64::from(height) - centre.1;
                let mut next = scale * dx.hypot(dy) / from;
                if snap {
                    next = (next * 20.0).round() / 20.0;
                }
                self.stage_guides.clear();
                if self.lanes.snap && *scale > 0.0 {
                    // The edges pull to the same lines a move pulls to - the
                    // frame's, and every other picture's - but here the pull
                    // sets the size, not the place: the scale that lands the
                    // nearest edge on its target, when one is inside reach.
                    let pull = 0.01 * f64::from(width.max(height));
                    let frame = (width, height);
                    let (cx, cy) = (centre.0 / f64::from(width), centre.1 / f64::from(height));
                    let mut xs = vec![0.0, 0.5, 1.0];
                    let mut ys = vec![0.0, 0.5, 1.0];
                    for other in self.stage_clips() {
                        if other.id == *clip {
                            continue;
                        }
                        let footprint = self.footprint(other);
                        let (ow, oh) = footprint.half_bounds(frame);
                        xs.extend([footprint.cx - ow, footprint.cx, footprint.cx + ow]);
                        ys.extend([footprint.cy - oh, footprint.cy, footprint.cy + oh]);
                    }
                    // Four edges: each is the centre plus or minus a half
                    // bound that grows with the scale.
                    let edges = [
                        (-1.0, half.0, cx, &xs, true, width),
                        (1.0, half.0, cx, &xs, true, width),
                        (-1.0, half.1, cy, &ys, false, height),
                        (1.0, half.1, cy, &ys, false, height),
                    ];
                    let mut best: Option<(f64, f64, bool, f64)> = None;
                    for (sign, base, at_centre, targets, vertical, extent) in edges {
                        if base <= 0.0 {
                            continue;
                        }
                        let edge = at_centre + sign * base * next / scale;
                        for &target in targets.iter() {
                            let distance = (target - edge).abs() * f64::from(extent);
                            let wanted = (target - at_centre) * sign;
                            if distance < pull
                                && wanted > 0.0
                                && best.is_none_or(|(held, ..)| distance < held)
                            {
                                best = Some((distance, scale * wanted / base, vertical, target));
                            }
                        }
                    }
                    if let Some((_, snapped, vertical, at)) = best {
                        next = snapped;
                        self.stage_guides.push(StageGuideData {
                            vertical,
                            at: at as f32,
                        });
                    }
                }
                let playhead = self.playhead;
                if let Some(clip) = self.echo_clip_mut(clip) {
                    let at = place_in(clip, playhead);
                    write_keyable(clip, model::KeyProperty::Scale, next.clamp(0.05, 8.0), at);
                }
            }
            Gesture::StageStretch {
                clip,
                across,
                stretch,
                centre,
                from,
                rotation,
            } => {
                let dx = x * f64::from(width) - centre.0;
                let dy = y * f64::from(height) - centre.1;
                let (sin, cos) = rotation.to_radians().sin_cos();
                let along = if *across {
                    dx * cos + dy * sin
                } else {
                    -dx * sin + dy * cos
                };
                let mut next = stretch * along.abs() / from;
                if snap {
                    next = (next * 20.0).round() / 20.0;
                }
                if let Some(clip) = self.echo_clip_mut(clip) {
                    if *across {
                        clip.stretch_x = next.clamp(0.1, 10.0);
                    } else {
                        clip.stretch_y = next.clamp(0.1, 10.0);
                    }
                }
            }
            Gesture::TextWidth {
                clip,
                centre,
                rotation,
            } => {
                let dx = x * f64::from(width) - centre.0;
                let dy = y * f64::from(height) - centre.1;
                let (sin, cos) = rotation.to_radians().sin_cos();
                let along = (dx * cos + dy * sin).abs();
                // Twice the reach, as a fraction of the frame; no narrower
                // than a few percent, or every word stands alone.
                let mut next = (2.0 * along / f64::from(width)).clamp(0.03, 2.0);
                if snap {
                    next = (next * 20.0).round() / 20.0;
                }
                if let Some(clip) = self.echo_clip_mut(clip) {
                    clip.text.get_or_insert_with(TextStyle::default).max_width = next;
                }
            }
            Gesture::TextHeight {
                clip,
                centre,
                rotation,
            } => {
                let dx = x * f64::from(width) - centre.0;
                let dy = y * f64::from(height) - centre.1;
                let (sin, cos) = rotation.to_radians().sin_cos();
                // The pointer's reach across the box's own axis.
                let across = (-dx * sin + dy * cos).abs();
                let mut next = (2.0 * across / f64::from(height)).clamp(0.03, 2.0);
                if snap {
                    next = (next * 20.0).round() / 20.0;
                }
                if let Some(clip) = self.echo_clip_mut(clip) {
                    clip.text.get_or_insert_with(TextStyle::default).max_height = next;
                }
            }
            Gesture::StageRotate {
                clip,
                rotation,
                centre,
                from,
            } => {
                let dx = x * f64::from(width) - centre.0;
                let dy = y * f64::from(height) - centre.1;
                let swept = (dy.atan2(dx) - from).to_degrees();
                let mut next = rotation + swept;
                if snap {
                    next = (next / 15.0).round() * 15.0;
                }
                // Kept in the inspector's range, wrapping rather than
                // stopping: a turn through the bottom carries on.
                next = (next + 180.0).rem_euclid(360.0) - 180.0;
                let playhead = self.playhead;
                if let Some(clip) = self.echo_clip_mut(clip) {
                    let at = place_in(clip, playhead);
                    write_keyable(clip, model::KeyProperty::Rotation, next, at);
                }
            }
            _ => {
                self.gesture = gesture;
                return;
            }
        }
        self.gesture = gesture;
        self.request_preview();
    }

    /// The stage gesture is over: whatever the echo moved becomes one
    /// transform command per picture, batched when there are several.
    pub fn stage_released(&mut self) {
        self.stage_guides.clear();
        let overlay = matches!(self.gesture, Gesture::Paint { .. })
            .then(|| self.stroke_overlay())
            .filter(|(path, _, _)| !path.is_empty());
        let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        let touched: Vec<String> = match gesture {
            Gesture::StageMove { origins, .. } => {
                origins.into_iter().map(|origin| origin.clip).collect()
            }
            Gesture::StageScale { clip, .. }
            | Gesture::StageRotate { clip, .. }
            | Gesture::StageStretch { clip, .. }
            | Gesture::TextWidth { clip, .. } => vec![clip],
            Gesture::Paint {
                clip,
                tool,
                size,
                at,
                points,
                ..
            } => {
                // The stroke becomes one command, and one undo step; a
                // smart stroke then has its thing read from the frame, and
                // stays drawn until it has been.
                let stroke = model::Stroke {
                    tool,
                    size,
                    points: points.clone(),
                    at: Some(at),
                };
                self.pending_stroke = stroke.is_smart().then_some(overlay).flatten();
                self.apply(Command::AddCutoutStroke {
                    clip_id: clip,
                    stroke,
                });
                self.ensure_regions();
                return;
            }
            other => {
                // Not ours to end; a lane gesture is still live.
                self.gesture = other;
                return;
            }
        };
        let Some(echo) = self.echo.take() else {
            return;
        };
        let mut commands = Vec::new();
        for id in touched {
            let (Some(after), Some(before)) = (
                echo.active().clip(&id),
                self.session
                    .as_ref()
                    .and_then(|session| session.project().active().clip(&id)),
            ) else {
                continue;
            };
            if after.scale != before.scale
                || after.offset_x != before.offset_x
                || after.offset_y != before.offset_y
                || after.rotation != before.rotation
                || after.stretch_x != before.stretch_x
                || after.stretch_y != before.stretch_y
            {
                commands.push(Command::SetClipTransform {
                    clip_id: id.clone(),
                    scale: Some(after.scale),
                    offset_x: Some(after.offset_x),
                    offset_y: Some(after.offset_y),
                    rotation: Some(after.rotation),
                    stretch_x: Some(after.stretch_x),
                    stretch_y: Some(after.stretch_y),
                });
            }
            // A title's wrap width, from its side grips.
            if after.text != before.text {
                commands.push(Command::UpdateClip {
                    clip_id: id,
                    patch: ClipPatch {
                        text: Some(after.text.clone()),
                        ..Default::default()
                    },
                });
            }
        }
        match commands.len() {
            0 => {
                // A click that moved nothing: the echo is gone and the
                // monitor goes back to the document.
                self.request_preview();
            }
            1 => {
                self.apply(commands.remove(0));
            }
            _ => {
                self.apply(Command::Batch { commands });
            }
        }
    }

    // ── cutouts ──
    //
    // A cutout is a mask per source instant, found by the host's model and
    // cached in the project folder. The window's part is to notice which
    // media need masks they do not have, run one analysis at a time, and
    // turn a brush on the stage into strokes on the document.

    /// Starts the analysis for the first media whose cutout wants masks
    /// that are not there, unless one is running. Called after every
    /// change; the finished job calls it again for whatever is next.
    pub fn ensure_cutouts(&mut self) {
        if !self.cutout_jobs.is_empty() || self.host.cutouts.is_busy() {
            return;
        }
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project = std::path::PathBuf::from(session.path());
        // One analysis per media and subject, keyed the way the job map is.
        let wanted: Vec<(String, AnalyseRequest)> = Cutouts::requests(self.project(), &project)
            .into_iter()
            .map(|(media_id, request)| (Self::analysis_key(&media_id, request.subject), request))
            .collect();
        let Some((id, request)) = wanted
            .into_iter()
            .find(|(_, request)| Cutouts::outstanding(request) > 0)
        else {
            return;
        };
        self.cutout_jobs.insert(id.clone(), (false, 0.0));
        let cutouts = Arc::clone(&self.host.cutouts);
        let epoch = crate::host::project_epoch();
        spawn_in_project(
            move || {
                let mut last = (false, -1.0f32);
                let reporting = id.clone();
                let result = cutouts.analyse(&request, &mut |progress| {
                    // Every percent, not every frame: the readout cannot
                    // use more and the event loop has other work.
                    let now = match progress {
                        concat_host::cutout::Progress::Fetching { received, total } => {
                            (true, received as f32 / total.max(1) as f32)
                        }
                        concat_host::cutout::Progress::Analysing(fraction) => (false, fraction),
                    };
                    if now.0 != last.0 || now.1 - last.1 >= 0.01 {
                        last = now;
                        let id = reporting.clone();
                        on_ui_in_project(epoch, move |studio, _, _| {
                            if let Some(held) = studio.cutout_jobs.get_mut(&id) {
                                *held = now;
                            }
                        });
                    }
                });
                (id, result)
            },
            |studio, _, _, (id, result)| {
                studio.cutout_jobs.remove(&id);
                match result {
                    Ok(_) => {
                        // The monitor shows the cut, and whatever else is
                        // waiting gets its turn.
                        studio.request_preview();
                        studio.ensure_cutouts();
                    }
                    Err(error) if error.contains("cancelled") => {}
                    Err(error) => studio.notify(&tf("Remove background: {0}", &[&error]), true),
                }
            },
        );
    }

    // ── enhance ──
    //
    // The restoration model reads a clip's file frame by frame and writes
    // an enhanced copy into the project's cache; the clip is then pointed
    // at the copy, as one undo step, and shows it wherever it showed the
    // original. The window's part is to start the job, keep the readout
    // moving, and re-point the clip when the copy is whole.

    /// Enhances the media of clip `id`: starts the job, unless one is
    /// running, and says so.
    /// Writes one clip's sound as it plays - trimmed, at its speed, with its
    /// level, fades and effects baked in - to a WAV in the project's audio
    /// folder, and puts the file in the bin under Generated › Processed. A
    /// copy, not a replacement: the clip keeps its media and its settings,
    /// and the file is independent of both, so it can be cut, moved and
    /// exported like any import.
    ///
    /// The export's own audio path does the work, on the clip alone: the
    /// project is flattened with just this clip on its timeline, the one
    /// export clip that comes out is moved to the origin, and the mixer
    /// writes it the way it writes an export's soundtrack.
    pub fn render_clip_sound(&mut self, id: &str) {
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        if !self.clip_has_sound(&clip) {
            self.notify(&t("This clip has no sound to render"), true);
            return;
        }
        let Some(project_dir) = self
            .session
            .as_ref()
            .map(|session| std::path::PathBuf::from(session.path()))
        else {
            return;
        };
        let mut alone = self.project().clone();
        alone.active_mut().clips.retain(|held| held.id == clip.id);
        let Some(mut piece) =
            concat_export::flatten::flatten_timeline_in(&alone, None, Some(&project_dir))
                .into_iter()
                .next()
        else {
            self.notify(&t("This clip has no sound to render"), true);
            return;
        };
        piece.start = 0.0;
        piece.muted = false;
        let duration = piece.duration;
        let pieces = concat_export::audio_pieces(&piece);

        let out_dir = project_dir.join("audio");
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0);
        let slug: String = clip
            .name
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .take(40)
            .collect();
        let file = out_dir.join(format!("processed-{slug}-{stamp}.wav"));
        let name = format!("{} · {}", clip.name, t("processed"));
        log::info!(
            "render: {} ({duration:.2}s, {} piece(s)) to {}",
            clip.name,
            pieces.len(),
            file.display()
        );
        self.notify(&t("Rendering the sound…"), false);
        spawn_in_project(
            move || -> Result<concat_host::media::MediaSummary, String> {
                std::fs::create_dir_all(&out_dir)
                    .map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;
                concat_media::audio::mix_to_file(&pieces, duration, &file)
                    .map_err(|error| error.to_string())?;
                concat_host::media::probe(&file.to_string_lossy())
            },
            move |studio, _, _, result| match result {
                Ok(summary) => {
                    let mut item = summary.to_new_media();
                    item.name = name;
                    item.origin = Some(model::MediaOrigin::Processed);
                    studio.apply(Command::AddMedia { item });
                    studio.notify(&t("Sound rendered to Generated › Processed"), false);
                }
                Err(error) => {
                    log::warn!("render: {error}");
                    studio.notify(&tf("Could not render the sound: {0}", &[&error]), true);
                }
            },
        );
    }

    pub fn enhance_clip(&mut self, id: &str) {
        if !self.enhance_jobs.is_empty() || self.host.enhancers.is_busy() {
            self.notify(&t("Enhance is already at work; one clip at a time"), true);
            return;
        }
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        if clip.kind != model::ClipKind::Video && clip.kind != model::ClipKind::Image {
            return;
        }
        let Some(media) = self
            .project()
            .media
            .iter()
            .find(|item| item.id == clip.media_id)
            .cloned()
        else {
            return;
        };
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project = std::path::PathBuf::from(session.path());
        let (width, height) = (media.width.unwrap_or(0), media.height.unwrap_or(0));
        if width == 0 || height == 0 {
            self.notify(&t("This file has no picture to enhance"), true);
            return;
        }
        let factor = concat_vision::enhance::factor_for(width, height);
        let still = clip.kind == model::ClipKind::Image;
        let Some(target) = concat_host::enhance::target_for(&project, &media.path, factor, still)
        else {
            self.notify(&tf("Could not read {0}", &[&media.path]), true);
            return;
        };
        let request = EnhanceRequest {
            media_path: media.path.clone(),
            still,
            factor,
            target,
        };
        let clip_id = clip.id.clone();
        self.enhance_jobs.insert(clip_id.clone(), (false, 0.0));
        self.notify(
            &tf(
                "Enhancing {0}: {1}× on its way, frame by frame",
                &[&media.name, &factor],
            ),
            false,
        );
        let enhancers = Arc::clone(&self.host.enhancers);
        let epoch = crate::host::project_epoch();
        spawn_in_project(
            move || {
                let mut last = (false, -1.0f32);
                let reporting = clip_id.clone();
                let result = enhancers.enhance(&request, &mut |progress| {
                    let now = match progress {
                        concat_host::enhance::Progress::Fetching { received, total } => {
                            (true, received as f32 / total.max(1) as f32)
                        }
                        concat_host::enhance::Progress::Analysing(fraction) => (false, fraction),
                    };
                    if now.0 != last.0 || now.1 - last.1 >= 0.01 {
                        last = now;
                        let id = reporting.clone();
                        on_ui_in_project(epoch, move |studio, _, _| {
                            if let Some(held) = studio.enhance_jobs.get_mut(&id) {
                                *held = now;
                            }
                        });
                    }
                });
                (clip_id, result)
            },
            |studio, _, _, (clip_id, result)| {
                studio.enhance_jobs.remove(&clip_id);
                match result {
                    Ok(path) => studio.adopt_enhanced(&clip_id, &path),
                    Err(error) if error.contains("cancelled") => {}
                    Err(error) => studio.notify(&tf("Enhance: {0}", &[&error]), true),
                }
            },
        );
    }

    /// Points clip `id` at the enhanced copy at `path`, probed the way an
    /// import is, and shows it.
    fn adopt_enhanced(&mut self, id: &str, path: &std::path::Path) {
        let summary = match media::probe(&path.to_string_lossy()) {
            Ok(summary) => summary,
            Err(error) => {
                self.notify(&tf("Enhance: {0}", &[&error]), true);
                return;
            }
        };
        let mut item = summary.to_new_media();
        // The bin names the copy after the original, marked, rather than
        // after its cache file.
        if let Some(original) = self
            .clip(id)
            .and_then(|clip| self.project().media.iter().find(|m| m.id == clip.media_id))
        {
            item.name = format!("{} (enhanced)", original.name);
        }
        self.apply(Command::ReplaceClipMedia {
            clip_id: id.to_owned(),
            item,
        });
        self.request_preview();
        self.notify(&t("Enhanced; the clip now shows the copy"), false);
    }

    /// The Mode row: 0 off, 1 automatic, 2 custom. The chroma rows are the
    /// inspector's own business, on the chain.
    pub fn cutout_mode(&mut self, mode: i32) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        let Some(clip) = self.clip(&id).cloned() else {
            return;
        };
        let held = clip.cutout.clone().unwrap_or_else(model::Cutout::auto);
        let cutout = match mode {
            1 => Some(model::Cutout {
                mode: model::CutoutMode::Auto,
                ..held
            }),
            2 => Some(model::Cutout {
                mode: model::CutoutMode::Custom,
                ..held
            }),
            _ => None,
        };
        if mode == 2 {
            self.painting = true;
        }
        if cutout != clip.cutout {
            self.apply(Command::SetClipCutout {
                clip_id: id,
                cutout,
            });
        }
    }

    /// The name an analysis runs under: the media and what it keeps.
    fn analysis_key(media_id: &str, subject: model::Subject) -> String {
        format!("{media_id}:{}", subject.key())
    }

    /// The Subject row: 0 automatic, 1 person, 2 object. A change means
    /// other masks, which the analysis notices on its own.
    pub fn cutout_subject(&mut self, index: i32) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        let Some(cutout) = self.clip(&id).and_then(|clip| clip.cutout.clone()) else {
            return;
        };
        let subject = match index {
            1 => model::Subject::Person,
            2 => model::Subject::Object,
            _ => model::Subject::Auto,
        };
        if cutout.subject == subject {
            return;
        }
        self.apply(Command::SetClipCutout {
            clip_id: id,
            cutout: Some(model::Cutout { subject, ..cutout }),
        });
    }

    /// Takes every stroke off the selected clip's cutout, keeping it custom.
    pub fn cutout_clear(&mut self) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        let Some(cutout) = self.clip(&id).and_then(|clip| clip.cutout.clone()) else {
            return;
        };
        if cutout.strokes.is_empty() {
            return;
        }
        self.apply(Command::SetClipCutout {
            clip_id: id,
            cutout: Some(model::Cutout {
                strokes: Vec::new(),
                ..cutout
            }),
        });
    }

    pub fn cutout_tool(&mut self, index: i32) {
        self.brush = index.clamp(0, BRUSHES.len() as i32 - 1) as usize;
    }

    pub fn cutout_size(&mut self, size: f32) {
        self.brush_size = f64::from(size).clamp(model::MIN_BRUSH, model::MAX_BRUSH);
    }

    pub fn cutout_painting(&mut self, on: bool) {
        self.painting = on;
        if !on {
            self.pending_stroke = None;
        }
        // The monitor's view changes with the brushes: tinted while they
        // are out, cut when they are put away.
        self.request_preview();
    }

    /// The picture a press on the stage would paint: the one selected clip,
    /// under the playhead, with a custom cutout, while painting is on.
    fn paint_target(&self) -> Option<Clip> {
        if !self.painting {
            return None;
        }
        let id = self.sole_selection()?;
        let clip = self.clip(&id)?;
        let custom = clip
            .cutout
            .as_ref()
            .is_some_and(|cutout| cutout.mode == model::CutoutMode::Custom);
        if !custom || !clip.kind.is_visual() || self.locked(&clip.track_id) {
            return None;
        }
        self.stage_clips()
            .into_iter()
            .any(|shown| shown.id == clip.id)
            .then(|| clip.clone())
    }

    /// Where a stage point lands in the source picture, in the fractions a
    /// stroke is stored in: the footprint's turn undone, then the crop and
    /// the flips, the same way a decoded pixel finds its mask.
    fn stage_to_source(&self, clip: &Clip, x: f64, y: f64) -> [f64; 2] {
        let footprint = self.footprint(clip);
        let (width, height) = self.output_size();
        let (width, height) = (f64::from(width.max(1)), f64::from(height.max(1)));
        let dx = (x - footprint.cx) * width;
        let dy = (y - footprint.cy) * height;
        let (sin, cos) = footprint.rotation.to_radians().sin_cos();
        let along = dx * cos + dy * sin;
        let down = -dx * sin + dy * cos;
        let px = along / (footprint.w * width).max(1e-6) + 0.5;
        let py = down / (footprint.h * height).max(1e-6) + 0.5;
        let mapping = concat_vision::Mapping {
            crop: clip
                .crop
                .map(|crop| {
                    [
                        crop.left as f32,
                        crop.top as f32,
                        crop.right as f32,
                        crop.bottom as f32,
                    ]
                })
                .unwrap_or([0.0; 4]),
            flip_h: clip.flip_h,
            flip_v: clip.flip_v,
        };
        let (u, v) = mapping.source_of(px as f32, py as f32);
        [f64::from(u), f64::from(v)]
    }

    /// The stroke in flight as the stage draws it: path commands over a
    /// 1000 × 1000 viewbox, the line's width as a fraction of the stage,
    /// and whether it is taking away. Empty between strokes.
    fn stroke_overlay(&self) -> (String, f32, bool) {
        let Gesture::Paint {
            clip,
            tool,
            size,
            screen,
            ..
        } = &self.gesture
        else {
            return self
                .pending_stroke
                .clone()
                .unwrap_or((String::new(), 0.0, false));
        };
        let Some((first, rest)) = screen.split_first() else {
            return (String::new(), 0.0, false);
        };
        let mut path = format!("M {:.1} {:.1}", first.0 * 1000.0, first.1 * 1000.0);
        // A single press still draws a dot: a line to where it already is,
        // with round caps.
        for (x, y) in rest.iter().chain(rest.is_empty().then_some(first)) {
            path.push_str(&format!(" L {:.1} {:.1}", x * 1000.0, y * 1000.0));
        }
        // The brush is `size` of the picture's width; on the stage that is
        // `size` of the picture's footprint.
        let width = self
            .clip(clip)
            .map(|clip| self.footprint(clip).w)
            .unwrap_or(1.0) as f32
            * *size as f32;
        let erase = matches!(
            tool,
            model::BrushTool::Eraser | model::BrushTool::SmartEraser
        );
        (path, width, erase)
    }

    // ── edits from menus and the tray ──

    /// Split at `at`: every selected clip the instant runs through, or every
    /// clip at all when nothing is selected.
    pub fn split_at(&mut self, at: f32, only_selected: bool) {
        let selection = self.selection.clone();
        let at = f64::from(at);
        let victims: Vec<String> = self
            .timeline()
            .clips
            .iter()
            .filter(|clip| {
                clip.start + f64::from(MIN_DURATION) < at
                    && at < clip.start + clip.duration - f64::from(MIN_DURATION)
                    && (!only_selected || selection.is_empty() || selection.contains(&clip.id))
                    && !self.locked(&clip.track_id)
            })
            .map(|clip| clip.id.clone())
            .collect();
        if !victims.is_empty() {
            self.apply(Command::SplitClips {
                clip_ids: victims,
                time: at,
            });
        }
    }

    /// Delete: the selection goes, and the hole stays unless the timeline
    /// is magnetic.
    pub fn delete_selected(&mut self) {
        let magnetic = self.prefs.magnetic;
        self.remove_selected(magnetic);
    }

    /// Ripple delete (⇧⌫): the selection goes and each lane closes behind
    /// it, so a rough cut needs no dragging-left after every deletion.
    /// https://github.com/jub0t/Concat/issues/106
    pub fn ripple_delete_selected(&mut self) {
        self.remove_selected(true);
    }

    fn remove_selected(&mut self, ripple: bool) {
        let doomed: Vec<String> = self
            .selection
            .iter()
            .filter(|id| {
                self.clip(id)
                    .is_some_and(|clip| !self.locked(&clip.track_id))
            })
            .cloned()
            .collect();
        if !doomed.is_empty() {
            self.apply(Command::RemoveClips {
                clip_ids: doomed,
                ripple,
            });
        }
        self.selection.clear();
    }

    pub fn merge_blocked(&self) -> Option<String> {
        if self.selection.len() != 2 {
            return Some(t("Select two clips to merge"));
        }
        why_not_merge(self.timeline(), &self.selection)
    }

    pub fn merge(&mut self) {
        if self.merge_blocked().is_some() {
            return;
        }
        let ids = self.selection.clone();
        let kept = ids[0].clone();
        self.apply(Command::MergeClips { clip_ids: ids });
        self.selection = if self.clip(&kept).is_some() {
            vec![kept]
        } else {
            Vec::new()
        };
    }

    /// A freeze frame at the playhead on a picture clip.
    ///
    /// Video extracts a JPEG still into the project cache; images reuse their
    /// media. The engine command splits the clip, inserts the hold, and
    /// ripples the rest of the lane.
    pub fn freeze_at_playhead(&mut self) {
        let at = f64::from(self.playhead);
        let edge = f64::from(MIN_DURATION);
        let target = self
            .menu_target
            .clone()
            .or_else(|| self.sole_selection())
            .and_then(|id| self.clip(&id).cloned())
            .filter(|clip| {
                (clip.kind == model::ClipKind::Video || clip.kind == model::ClipKind::Image)
                    && !self.locked(&clip.track_id)
                    && at > clip.start + edge
                    && at < clip.start + clip.duration - edge
            });
        let Some(clip) = target else {
            self.notify("Park the playhead inside a picture clip to freeze", true);
            return;
        };

        let still = if clip.kind == model::ClipKind::Video {
            let Some(media) = self
                .project()
                .media
                .iter()
                .find(|item| item.id == clip.media_id)
            else {
                return;
            };
            let Some(session) = self.session.as_ref() else {
                return;
            };
            let project_path = session.path().to_owned();
            let source_time = clip.source_start + (at - clip.start) * clip.speed;
            let frame = match media::still_at(&media.path, source_time, 1280) {
                Ok(frame) => frame,
                Err(error) => {
                    self.notify(&error, true);
                    return;
                }
            };
            let bytes = match jpeg(&frame, 4) {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.notify(&format!("{error}"), true);
                    return;
                }
            };
            let key = format!(
                "freeze-{}-{}.jpg",
                clip.id,
                (source_time * 1000.0).round() as i64
            );
            if let Err(error) = media::write_artwork(&project_path, &key, &bytes) {
                self.notify(&error, true);
                return;
            }
            let path = format!("{project_path}/cache/{key}");
            match media::probe(&path) {
                Ok(summary) => Some(summary.to_new_media()),
                Err(error) => {
                    self.notify(&error, true);
                    return;
                }
            }
        } else {
            None
        };

        let created = self.apply(Command::FreezeFrame {
            clip_id: clip.id,
            time: at,
            duration: Some(1.0),
            still,
        });
        if let Some(id) = created {
            self.selection = vec![id];
        }
    }

    /// A copy of `source` laid after it. Three commands, because a clip's
    /// in-point and length are set by trims, not by placement.
    /// Duplicate every unlocked selected clip (right-to-left by start so
    /// neighbours do not stack). Falls back to the menu target when the
    /// selection is empty.
    pub fn duplicate_selected(&mut self) {
        let mut sources: Vec<Clip> = self
            .selection
            .iter()
            .filter_map(|id| self.clip(id).cloned())
            .filter(|clip| !self.locked(&clip.track_id))
            .collect();
        #[allow(clippy::collapsible_if)]
        if sources.is_empty() {
            if let Some(id) = self.menu_target.clone() {
                if let Some(clip) = self.clip(&id).cloned() {
                    if !self.locked(&clip.track_id) {
                        sources.push(clip);
                    }
                }
            }
        }
        if sources.is_empty() {
            return;
        }
        sources.sort_by(|left, right| right.start.total_cmp(&left.start));
        let mut last = None;
        for source in &sources {
            self.duplicate(source);
            last = self.selection.first().cloned();
        }
        if let Some(id) = last {
            self.selection = vec![id];
        }
    }

    pub fn duplicate(&mut self, source: &Clip) {
        let end = source.start + source.duration;
        if source.kind == model::ClipKind::Text {
            let created = self.apply(Command::AddTextClip {
                above: false,
                track_id: Some(source.track_id.clone()),
                start: end,
                style: source.text.clone(),
                duration: Some(source.duration),
                offset_y: Some(source.offset_y),
            });
            if let Some(id) = created {
                self.selection = vec![id];
            }
            return;
        }
        let Some(created) = self.apply(Command::AddClip {
            media_id: source.media_id.clone(),
            track_id: source.track_id.clone(),
            start: (end - source.source_start / source.speed).max(0.0),
            ripple: false,
        }) else {
            return;
        };
        let placed = self.clip(&created).cloned();
        let Some(placed) = placed else { return };
        let mut commands = Vec::new();
        let head = end - placed.start;
        if head.abs() > 1e-6 {
            commands.push(Command::TrimClip {
                clip_id: created.clone(),
                edge: TrimEdge::Start,
                delta: head,
                ripple: false,
            });
        }
        let after_head = placed.duration - head.max(0.0);
        let tail = source.duration - after_head;
        if tail.abs() > 1e-6 {
            commands.push(Command::TrimClip {
                clip_id: created.clone(),
                edge: TrimEdge::End,
                delta: tail,
                ripple: false,
            });
        }
        commands.push(Command::UpdateClip {
            clip_id: created.clone(),
            patch: ClipPatch {
                name: Some(format!("{} copy", source.name)),
                volume: Some(source.volume),
                fade_in: Some(source.fade_in),
                fade_out: Some(source.fade_out),
                opacity: Some(source.opacity),
                preserve_pitch: Some(source.preserve_pitch),
                filters: Some(source.filters.clone()),
                video_effects: Some(source.video_effects.clone()),
                ..ClipPatch::default()
            },
        });
        self.apply(Command::Batch { commands });
        self.selection = vec![created];
    }

    /// What the tray's sound and word tools may do to the selection: one
    /// clip with sound for Captions to start on, one title for Speak to
    /// read. Hints, not gates - both sheets open without them.
    fn sound_tools(&self) -> (bool, bool) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return (false, false);
        };
        (
            self.clip_has_sound(clip),
            clip.kind == model::ClipKind::Text,
        )
    }

    /// Where a clip's sound is: whether it is a video clip whose sound is
    /// still on its picture, to take off, and whether it is a picture whose
    /// sound is off it - or that sound itself - to put back.
    fn sound_placement(&self, clip: &Clip) -> (bool, bool) {
        let detached = self
            .timeline()
            .clips
            .iter()
            .any(|other| other.detached_from.as_deref() == Some(clip.id.as_str()));
        let video = clip.kind == model::ClipKind::Video;
        (
            video && !detached,
            (video && detached)
                || (clip.kind == model::ClipKind::Audio && clip.detached_from.is_some()),
        )
    }

    /// Whether the clip has sound to transcribe: an audio clip, or a video
    /// clip whose file carries an audio stream. A silent video is not a
    /// sound source, however much it looks like one.
    pub(crate) fn clip_has_sound(&self, clip: &Clip) -> bool {
        match clip.kind {
            model::ClipKind::Audio => true,
            model::ClipKind::Video => self
                .project()
                .media_by_id(&clip.media_id)
                .is_some_and(|media| media.has_audio),
            _ => false,
        }
    }

    // ── projects ──

    /// Opens a project as the session and leaves the launch screen, or
    /// says why it could not.
    pub fn open_project(&mut self, info: ProjectInfo) -> Result<(), String> {
        if self
            .host
            .open_projects
            .claim(&info.path, concat_api::Holder::Window)
            .is_err()
        {
            return Err(t("This project is open through the Remote API"));
        }
        match Session::open_info(&info) {
            Ok(session) => {
                if let Err(error) = projects::remember(&self.host.dirs.config, &info) {
                    log::warn!("{error}");
                }
                self.pause();
                // The proxies of media no longer in the project, or read
                // as a range they no longer are, go now: nothing else
                // sweeps cache/proxy.
                let project_dir = std::path::Path::new(session.path());
                let kept: HashSet<std::path::PathBuf> = session
                    .project()
                    .media
                    .iter()
                    .filter_map(|item| {
                        concat_host::proxy::path_for(
                            project_dir,
                            &item.path,
                            item.color_range.map(concat_export::engine_range),
                        )
                    })
                    .collect();
                let swept = concat_host::proxy::sweep(project_dir, &kept);
                if swept > 0 {
                    log::info!("{swept} stale proxies swept from {}", project_dir.display());
                }
                self.session = Some(session);
                crate::host::next_project_epoch();
                self.art_failed.clear();
                self.art_stamp = None;
                self.proxied.clear();
                self.echo = None;
                self.dirty = false;
                self.project_name = info.name.clone();
                self.export.name = projects::folder_name(&info.name);
                self.selection.clear();
                self.media.selected.clear();
                self.playhead = 0.0;
                self.handle(crate::panes::Msg::Timeline(
                    crate::panes::timeline::TimelineMsg::Reset,
                ));
                self.on_start = false;
                self.handle(crate::panes::Msg::Monitor(
                    crate::panes::monitor::MonitorMsg::Opened,
                ));
                self.recents = projects::list(&self.host.dirs.config);
                self.host.monitor.clear();
                self.audition = None;
                self.revision += 1;
                self.flat = None;
                self.sync_audio();
                self.request_media_art();
                self.request_preview();
                self.ensure_cutouts();
                self.ensure_regions();

                // Log missing media to file for debugging
                if let Some(session) = &self.session {
                    let missing = session.project().missing_media();
                    if !missing.is_empty() {
                        let log_path = std::path::Path::new(&info.path)
                            .join("cache")
                            .join("missing_media.log");
                        if let Ok(mut file) = std::fs::File::create(&log_path) {
                            use std::io::Write;
                            let _ = writeln!(file, "{} media files missing:", missing.len());
                            for m in &missing {
                                let _ = writeln!(file, "  - {} ({})", m.name, m.path);
                            }
                        }

                        self.handle(crate::panes::Msg::Relink(
                            crate::panes::relink::RelinkMsg::Show(missing),
                        ));
                    }
                }
                Ok(())
            }
            Err(error) => {
                self.host.open_projects.release(&info.path);
                Err(error)
            }
        }
    }

    /// Clears all cached artwork and waveforms from the current project.
    pub fn clear_project_cache(&mut self) {
        let Some(path) = self
            .session
            .as_ref()
            .map(|session| session.path().to_string())
        else {
            return;
        };
        match concat_host::projects::clear_cache(&path) {
            Ok(count) => self.notify(&format!("Cleared {count} cache files"), false),
            Err(error) => self.notify(&format!("Cache clear failed: {error}"), true),
        }
        self.request_media_art();
        self.request_preview();
    }

    /// Saves, then closes the session and returns to the launch screen.
    pub fn close_project(&mut self) {
        // Words still being typed land in the save, not on the floor.
        self.flush_commit();
        self.pause();
        self.host.cutouts.cancel();
        self.host.enhancers.cancel();
        self.enhance_jobs.clear();
        self.cutout_jobs.clear();
        self.region_job = None;
        if let Some(session) = self.session.as_mut() {
            let (path, document) = session.prepare_save(None);
            if let Err(error) = projects::save(&path, &document) {
                self.notify(&tf("Could not save: {0}", &[&error]), true);
                return;
            }
        }
        self.autosave.stop();
        if let Some(session) = self.session.as_ref() {
            self.host.open_projects.release(session.path());
        }
        self.session = None;
        // Whatever a worker still brings back for this project is dropped
        // at delivery; the sheets that were waiting on one stop waiting.
        crate::host::next_project_epoch();
        self.art_failed.clear();
        self.art_stamp = None;
        self.proxied.clear();
        self.captions.running = false;
        self.captions.progress = 0.0;
        self.speech.running = false;
        self.speech.progress = 0.0;
        self.echo = None;
        self.dirty = false;
        self.selection.clear();
        self.gesture = Gesture::None;
        self.handle(crate::panes::Msg::Monitor(
            crate::panes::monitor::MonitorMsg::Closed,
        ));
        self.host.monitor.clear();
        self.audition = None;
        self.revision += 1;
        self.flat = None;
        self.host
            .playback
            .set_clips(std::path::PathBuf::new(), Vec::new());
        self.on_start = true;
        self.recents = projects::list(&self.host.dirs.config);
    }

    /// Posters for the recents that have none yet, decoded on a worker.
    fn request_posters(&mut self) {
        let wanted: Vec<String> = self
            .recents
            .iter()
            .map(|project| project.path.clone())
            .filter(|path| !self.posters.contains_key(path) && !self.posters_pending.contains(path))
            .collect();
        for path in wanted {
            self.posters_pending.insert(path.clone());
            spawn(
                move || {
                    let cached = media::poster_cache(&path);
                    // A project with no poster - nothing on it, or nothing
                    // but black - keeps the film mark; see `poster_frame`.
                    let made = media::poster_frame(&path).is_ok();
                    (path, made.then_some(cached))
                },
                |studio, _, _, (path, cached)| {
                    studio.posters_pending.remove(&path);
                    if let Some(poster) = cached.and_then(|cached| image_at(&cached)) {
                        studio.posters.insert(path, poster);
                    }
                },
            );
        }
    }

    /// Routes one message to its pane and applies it. The pane is taken out
    /// of the studio for the duration, so its `update` can be handed the
    /// rest of the window without borrowing itself twice; a pane never
    /// reads its own slot on the studio.
    pub fn handle(&mut self, msg: crate::panes::Msg) {
        match msg {
            crate::panes::Msg::Export(msg) => {
                let mut pane = std::mem::take(&mut self.export);
                pane.update(msg, self);
                self.export = pane;
            }
            crate::panes::Msg::Settings(msg) => {
                let mut pane = std::mem::take(&mut self.settings);
                pane.update(msg, self);
                self.settings = pane;
            }
            crate::panes::Msg::Captions(msg) => {
                let mut pane = std::mem::take(&mut self.captions);
                pane.update(msg, self);
                self.captions = pane;
            }
            crate::panes::Msg::Speech(msg) => {
                let mut pane = std::mem::take(&mut self.speech);
                pane.update(msg, self);
                self.speech = pane;
            }
            crate::panes::Msg::Relink(msg) => {
                let mut pane = std::mem::take(&mut self.relink);
                pane.update(msg, self);
                self.relink = pane;
            }
            crate::panes::Msg::Project(msg) => {
                let mut pane = std::mem::take(&mut self.project_sheet);
                pane.update(msg, self);
                self.project_sheet = pane;
            }
            crate::panes::Msg::Start(msg) => {
                let mut pane = std::mem::take(&mut self.start);
                pane.update(msg, self);
                self.start = pane;
            }
            crate::panes::Msg::Media(msg) => {
                let mut pane = std::mem::take(&mut self.media);
                pane.update(msg, self);
                self.media = pane;
            }
            crate::panes::Msg::Monitor(msg) => {
                let mut pane = std::mem::take(&mut self.monitor);
                pane.update(msg, self);
                self.monitor = pane;
            }
            crate::panes::Msg::Timeline(msg) => {
                let mut pane = std::mem::take(&mut self.lanes);
                pane.update(msg, self);
                self.lanes = pane;
            }
        }
    }

    // ── speech ──

    /// Packs the open project into the template library.
    /// Where the user's own looks live: one package folder each.
    pub fn looks_dir(dirs: &concat_host::dirs::AppDirs) -> std::path::PathBuf {
        dirs.config.join("effects")
    }

    /// Rebuilds the catalogue from the built-ins and the packages in
    /// `looks_dir`, and says in the window what would not load: the first
    /// reason and how many more, with every reason in the log, so an
    /// author sees a manifest or shader fault where they are looking
    /// rather than in a file they may not know exists. Returns how many
    /// failed. With `announce`, a clean reload is reported too, with the
    /// folder, which is how a newcomer learns where packages go.
    pub fn reload_packages(&mut self, announce: bool) -> usize {
        /// A custom package's shader runs once over a picture this many
        /// pixels a side before the package is offered: enough that a loop
        /// bounded in the thousands per pixel shows, and a real frame's
        /// worth of work does not.
        const TRIAL_SIDE: u32 = 512;
        /// How long that trial may take before the package is refused.
        const TRIAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
        let dir = Self::looks_dir(&self.host.dirs);
        let monitor = &self.host.monitor;
        let errors = Catalogue::install_with(&dir, &mut |package| {
            let Some(pass) = package.trial_pass() else {
                return Ok(());
            };
            match monitor.trial(&pass, TRIAL_SIDE, TRIAL_TIMEOUT) {
                Some(Err(why)) => Err(format!("its shader failed its trial: {why}")),
                _ => Ok(()),
            }
        });
        for error in &errors {
            log::warn!("package: {error}");
        }
        // A card for every package of the user's own that has none and
        // can have one. A card that fails is a line in the log, not a
        // notice: the package itself loaded.
        let catalogue = Catalogue::builtin();
        for package in catalogue.packages() {
            let Some(folder) = package.folder.as_deref() else {
                continue;
            };
            if !package.kind().is_visual()
                || package.kind() == PackageKind::Transition
                || folder.join("preview.png").is_file()
                || folder.join("preview.jpg").is_file()
            {
                continue;
            }
            let chain = catalogue.video_chain(&[AppliedFilter::new(package.id().to_owned())]);
            if chain.is_empty() {
                continue;
            }
            if let Err(error) = render_card(&dir, package, &chain) {
                log::warn!("package {}: card: {error}", package.id());
            }
        }
        // A changed shader means a changed card: the stills are read again.
        self.look_art.borrow_mut().clear();
        // The folder as the watch will next see it, cards included, so a
        // reload does not read as a change and set off another.
        self.packages_seen = concat_effects::package_stamp(&dir);
        self.packages_pending = None;
        if let Some(first) = errors.first() {
            let message = if errors.len() == 1 {
                tf("A custom package did not load: {0}", &[first])
            } else {
                tf(
                    "{0} custom packages did not load; the first: {1}",
                    &[&errors.len(), first],
                )
            };
            self.notify(&message, true);
        } else if announce {
            let count = Catalogue::builtin()
                .packages()
                .filter(|package| package.folder.is_some())
                .count();
            self.notify(
                &tf(
                    "Loaded {0} custom package(s) from {1}",
                    &[&count, &dir.display()],
                ),
                false,
            );
        }
        errors.len()
    }

    /// Starts the watch on the effects folder: a poll every two seconds,
    /// and a reload once two polls in a row see the same changed folder.
    /// A package written or dropped in while the app runs then shows up on
    /// its own, and one that will not load says why, without a restart or
    /// the button.
    pub fn watch_packages(&mut self) {
        self.packages_watch.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(2),
            || {
                crate::host::Shell::with(|shell, app| {
                    let changed = shell.studio.borrow_mut().poll_packages();
                    if changed {
                        shell.studio.borrow_mut().refresh_art();
                        shell.studio.borrow().publish(&app, &shell.models);
                    }
                });
            },
        );
    }

    /// One tick of the watch: whether the catalogue was rebuilt.
    fn poll_packages(&mut self) -> bool {
        let now = concat_effects::package_stamp(&Self::looks_dir(&self.host.dirs));
        if now == self.packages_seen {
            self.packages_pending = None;
            return false;
        }
        if self.packages_pending != Some(now) {
            // Seen once; a copy may still be in progress. Next tick decides.
            self.packages_pending = Some(now);
            return false;
        }
        self.reload_packages(true);
        true
    }

    /// Imports one or more `.cube` tables as looks: each becomes a package
    /// folder under `looks_dir` - a manifest that names the table and a
    /// shader that reads it - with a card still rendered through the table
    /// here, and the catalogue is rebuilt so the Filters page shows them.
    pub fn import_lut(&mut self) {
        let Some(paths) =
            crate::platform::pick_files(&t("Import LUT"), Some((t("LUT").as_str(), &["cube"])))
        else {
            return;
        };
        let dir = Self::looks_dir(&self.host.dirs);
        let mut imported = 0;
        for path in paths {
            match import_cube(&dir, &path) {
                Ok(id) => {
                    self.look_art.borrow_mut().remove(&id);
                    imported += 1;
                }
                Err(error) => {
                    self.notify(
                        &tf("Could not import {0}: {1}", &[&path.display(), &error]),
                        true,
                    );
                }
            }
        }
        if imported == 0 {
            return;
        }
        // A look that will not load has its reason on screen; the count of
        // the rest would only cover it.
        if self.reload_packages(false) > 0 {
            return;
        }
        self.library[0].query.clear();
        self.notify(
            &tf(
                "Imported {0} look(s); find them under Imported",
                &[&imported],
            ),
            false,
        );
    }

    pub fn save_template(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let document = session.document();
        let settings = session.settings();
        let path = session.path().to_owned();
        let name = format!("{} template", self.project_name);
        let config = self.host.dirs.config.clone();
        spawn(
            move || templates::save(&config, &document, &settings, &path, &name),
            |studio, _, _, result| match result {
                Ok(info) => studio.notify(&tf("Saved template “{0}”", &[&info.name]), false),
                Err(error) => studio.notify(&error, true),
            },
        );
    }

    // ── notices ──

    /// Raises the bottom-right notice. Every caller is some other handler
    /// that has just done - or refused to do - the thing it is reporting, so
    /// this only records; the publish that handler was going to make anyway
    /// is what puts it on screen.
    pub fn notify(&mut self, message: &str, failed: bool) {
        self.toast.token += 1;
        self.toast.message = message.into();
        self.toast.failed = failed;
    }

    // ── publishing ──

    pub fn publish(&self, app: &App, models: &Models) {
        self.publish_lanes(app, models);
        self.publish_chrome(app, models);
        self.publish_dock(app, models);
    }

    pub fn publish_dock(&self, _app: &App, models: &Models) {
        let out = self.dock_layout();
        sync(&models.seats, out.seats);
        sync(&models.dividers, out.dividers);
    }

    /// Shows the compact dock, or the wide one, keeping whichever is put
    /// away whole; see `COMPACT_WIDTH`.
    pub fn set_compact(&mut self, compact: bool) {
        if compact != self.compact {
            std::mem::swap(&mut self.dock, &mut self.dock_aside);
            self.compact = compact;
        }
    }

    pub fn dock_layout(&self) -> DockLayout {
        let mut out = DockLayout::default();
        let (width, height) = self.workspace;
        if width > SEAT_GAP * 2.0 && height > SEAT_GAP * 2.0 {
            lay_out(
                &self.dock,
                (
                    SEAT_GAP,
                    SEAT_GAP,
                    width - 2.0 * SEAT_GAP,
                    height - 2.0 * SEAT_GAP,
                ),
                &mut out,
            );
        }
        out
    }

    pub fn split_extent(&self, index: usize) -> Option<f32> {
        self.dock_layout().extents.get(index).copied()
    }

    pub fn split_ratio(&self, index: usize) -> Option<f32> {
        match self.dock.at(&self.dock.split_path(index)?) {
            Dock::Split { ratio, .. } => Some(*ratio),
            _ => None,
        }
    }

    /// The timeline and the readouts that follow it: what runs on every
    /// event of a scrub, a drag, a trim or a knob.
    pub fn publish_lanes(&self, app: &App, models: &Models) {
        let editor = app.global::<Editor>();
        let project = self.project();
        sync(
            &models.tabs,
            project
                .timelines
                .iter()
                .map(|timeline| TimelineTabData {
                    id: timeline.id.as_str().into(),
                    name: timeline.name.as_str().into(),
                })
                .collect(),
        );

        let timeline = self.timeline();
        let mut top = 0.0;
        sync(
            &models.tracks,
            timeline
                .tracks
                .iter()
                .rev()
                .map(|lane| {
                    let height = self.lane_height(lane);
                    let row = TrackData {
                        id: lane.id.as_str().into(),
                        visible: lane.visible,
                        muted: lane.muted,
                        locked: self.locked(&lane.id),
                        size: self.lane_size(&lane.id),
                        height,
                        top,
                    };
                    top += height;
                    row
                })
                .collect(),
        );

        // Only the clips near the view: what the lanes show plus a screen
        // either side; see `TimelinePane::published_span`.
        let span = self.lanes.published_span();
        sync(
            &models.clips,
            timeline
                .clips
                .iter()
                .filter(|clip| {
                    crate::panes::timeline::TimelinePane::shows(
                        span,
                        clip.start as f32,
                        clip.duration as f32,
                    )
                })
                .map(|clip| {
                    let wave =
                        if matches!(clip.kind, model::ClipKind::Audio | model::ClipKind::Video) {
                            self.wave(clip)
                        } else {
                            Default::default()
                        };
                    ClipData {
                        id: clip.id.as_str().into(),
                        name: clip.name.as_str().into(),
                        kind: kind_of(clip),
                        row: self.row_of(&clip.track_id),
                        start: clip.start as f32,
                        duration: clip.duration as f32,
                        selected: self.selection.iter().any(|id| id == &clip.id),
                        fx: clip.video_effects.iter().any(|effect| effect.enabled),
                        transition_duration: clip
                            .transition_in
                            .as_ref()
                            .map(|transition| transition.duration as f32)
                            .unwrap_or(0.0),
                        fade_in: clip.fade_in as f32,
                        fade_out: clip.fade_out as f32,
                        volume: clip.volume as f32,
                        text_body: clip
                            .text
                            .as_ref()
                            .map(|text| text.content.as_str())
                            .unwrap_or_default()
                            .into(),
                        wave: wave.0,
                        wave_from: wave.1,
                        wave_span: wave.2,
                        strip: self.strip_of(clip),
                    }
                })
                .collect(),
        );

        // What is under the playhead. The topmost picture wins, the way the
        // compositor stacks them.
        let playhead = f64::from(self.playhead);
        let showing = timeline
            .clips
            .iter()
            .filter(|clip| {
                (clip.kind.is_visual() || clip.kind == model::ClipKind::Text)
                    && clip.start <= playhead
                    && playhead < clip.start + clip.duration
            })
            .max_by_key(|clip| -self.row_of(&clip.track_id));
        editor.set_has_picture(showing.is_some());
        editor.set_preview_clip_name(
            showing
                .map(|clip| clip.name.as_str())
                .unwrap_or_default()
                .into(),
        );
        editor.set_preview_duration(self.duration());
        editor.set_playhead_free(!self.prefs.playhead_stops_at_end);
        editor.set_playing(self.playing);
        editor.set_preview_frame(self.monitor.image.clone());
        sync(&models.stage, self.stage_items());
        sync(&models.guides, self.stage_guides.clone());
        let (path, width, erase) = self.stroke_overlay();
        editor.set_stroke_path(path.into());
        editor.set_stroke_width(width);
        editor.set_stroke_erase(erase);
        editor.set_brush_tool(self.brush as i32);
        editor.set_brush_size(self.brush_size as f32);
        editor.set_painting(self.painting);

        editor.set_drop(match &self.drop {
            Some(plan) => DropData {
                active: true,
                kind: plan.kind,
                label: plan.label.as_str().into(),
                start: plan.start,
                duration: plan.duration,
                row: plan.row,
            },
            None => DropData::default(),
        });

        editor.set_selected_clip(self.selected());
        // Written only when it differs: a fresh model is a change to every
        // binding that reads it, and this one is read on every publish.
        let labels = self.audio_track_labels();
        let shown = editor.get_audio_tracks();
        let same = shown.row_count() == labels.len()
            && labels
                .iter()
                .enumerate()
                .all(|(row, label)| shown.row_data(row).as_ref() == Some(label));
        if !same {
            editor.set_audio_tracks(slint::ModelRc::new(VecModel::from(labels)));
        }
        // The keyframe cluster's state, on its own global: every keyable row
        // in the inspector reads it, and none of them is threaded to.
        let keys = app.global::<Keyframes>();
        // The Keyframes panel: set whole, since a row holds a model of its
        // keys and cannot be compared for a diff.
        models.key_editor_rows.set_vec(self.key_editor_rows());
        app.global::<KeyEditor>().set_at(
            self.sole_selection()
                .and_then(|id| self.clip(&id))
                .map_or(0.0, |clip| place_in(clip, self.playhead) as f32),
        );
        let rows = self.key_rows();
        keys.set_available(!rows.is_empty());
        sync(&models.key_rows, rows);
        editor.set_inspector_jump_token(self.inspector_jump.0);
        editor.set_library_audition(self.audition_of().unwrap_or("").into());
        editor.set_inspector_jump_tab(self.inspector_jump.1.into());
        editor.set_inspector_jump_section(self.inspector_jump.2.into());
        let active = project
            .timelines
            .iter()
            .position(|timeline| timeline.id == project.active_timeline_id)
            .unwrap_or(0);
        editor.set_timeline_current_tab(active as i32);
        editor.set_playhead(self.playhead);
        sync(&models.key_marks, self.key_marks());
        editor.set_scroll_left(self.lanes.scroll_left);
        editor.set_seconds_per_pixel(self.lanes.seconds_per_pixel);
        editor.set_frame_rate(self.frame_rate());
        editor.set_tool(self.lanes.tool);
        editor.set_snap(self.lanes.snap);
        editor.set_magnetic(self.prefs.magnetic);
        editor.set_preview_axis(self.prefs.preview_axis);
        editor.set_preview_axis_audio(self.prefs.preview_axis_audio);
        editor.set_pan_mode(self.lanes.pan_mode);
        editor.set_selected_count(self.selection.len() as i32);
        // The tray's undo and redo buttons grey out on these; the Edit menu
        // asks the session itself when it opens.
        // https://github.com/jub0t/Concat/issues/154
        let (can_undo, can_redo) = self.session.as_ref().map_or((false, false), |session| {
            (session.can_undo(), session.can_redo())
        });
        editor.set_can_undo(can_undo);
        editor.set_can_redo(can_redo);
        let (sound_selected, title_selected) = self.sound_tools();
        editor.set_sound_selected(sound_selected);
        editor.set_title_selected(title_selected);
        editor.set_merge_blocked_because(match self.merge_blocked() {
            Some(reason) => reason.into(),
            None => SharedString::new(),
        });

        // The selected clip's chains, as the inspector's two stacks.
        let (video, audio) = match self.sole_selection().and_then(|id| self.clip(&id)) {
            Some(clip) => (
                clip.video_effects
                    .iter()
                    .enumerate()
                    .map(|(index, effect)| EffectData {
                        id: index as i32 + 1,
                        name: label_of(&effect.id).into(),
                        audio: false,
                    })
                    .collect(),
                clip.filters
                    .iter()
                    .enumerate()
                    .map(|(index, filter)| EffectData {
                        id: index as i32 + 1000,
                        name: label_of(&filter.id).into(),
                        audio: true,
                    })
                    .collect(),
            ),
            None => (Vec::new(), Vec::new()),
        };
        sync(&models.video_effects, video);
        sync(&models.audio_effects, audio);

        // The same chains as the inspector's stacks see them: a row per
        // link and a row per knob, from the catalogue's manifests.
        let (visual, visual_params, sound, sound_params) =
            match self.sole_selection().and_then(|id| self.clip(&id)) {
                Some(clip) => {
                    let (visual, visual_params) = chain_rows(&clip.video_effects);
                    let (sound, sound_params) = chain_rows(&clip.filters);
                    (visual, visual_params, sound, sound_params)
                }
                None => (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
            };
        sync(&models.applied_visual, visual);
        sync(&models.visual_params, visual_params);
        sync(&models.applied_audio, sound);
        sync(&models.audio_params, sound_params);
        sync(
            &models.adjust_params,
            match self.sole_selection().and_then(|id| self.clip(&id)) {
                Some(clip) if clip.kind.is_visual() => {
                    adjust_rows(&clip.video_effects, self.key_point().map(|(_, at)| at))
                }
                _ => Vec::new(),
            },
        );
    }

    // ── the effect libraries ──

    /// The view state of one library, or None for a shelf index the panel
    /// should never have sent.
    fn library_at(&mut self, shelf: i32) -> Option<&mut LibraryView> {
        self.library.get_mut(usize::try_from(shelf).ok()?)
    }

    /// The search box was typed in. A live query searches every shelf, so
    /// the strip is put away while one is running - see `shelves`.
    pub fn library_query(&mut self, shelf: i32, text: &str) {
        if let Some(view) = self.library_at(shelf) {
            view.query = text.to_owned();
        }
    }

    /// A shelf was picked. Typing and then picking a shelf means the shelf,
    /// so the query goes.
    pub fn library_group(&mut self, shelf: i32, index: i32) {
        if let Some(view) = self.library_at(shelf) {
            view.group = index.max(0);
            view.category.clear();
            view.query.clear();
            view.favourites = false;
        }
    }

    /// A category pill was picked.
    pub fn library_category(&mut self, shelf: i32, category: &str) {
        if let Some(view) = self.library_at(shelf) {
            view.category = category.to_owned();
            view.group = -1;
            view.query.clear();
        }
    }

    /// The star chip. Turning it on clears the query for the same reason
    /// picking a shelf does: two filters at once is a list nobody can
    /// predict the contents of.
    pub fn library_favourites(&mut self, shelf: i32, on: bool) {
        if let Some(view) = self.library_at(shelf) {
            view.favourites = on;
            view.query.clear();
        }
    }

    /// Star or unstar a package, and remember it. Not per library: a star is
    /// a fact about the package.
    pub fn library_favourite(&mut self, id: &str, on: bool) {
        let held = self.prefs.favourites.iter().any(|held| held == id);
        if on && !held {
            self.prefs.favourites.push(id.to_owned());
        } else if !on && held {
            self.prefs.favourites.retain(|held| held != id);
        } else {
            return;
        }
        self.prefs.save(&self.host.dirs);
    }

    // ── keyframes ──

    /// The keyable property a field names, or None for a field that is a
    /// constant and stays one.
    pub fn key_property_of(field: ClipField) -> Option<model::KeyProperty> {
        Some(match field {
            ClipField::Scale => model::KeyProperty::Scale,
            ClipField::OffsetX => model::KeyProperty::OffsetX,
            ClipField::OffsetY => model::KeyProperty::OffsetY,
            ClipField::Rotation => model::KeyProperty::Rotation,
            ClipField::Opacity => model::KeyProperty::Opacity,
            ClipField::Volume => model::KeyProperty::Volume,
            _ => return None,
        })
    }

    /// The selected clip and where the playhead is inside it, `0..=1`, or
    /// None when there is no sole selection or the playhead is outside it.
    ///
    /// Outside is not clamped to an end: a key put on from outside the clip
    /// would land on its first or last frame, which is never what the person
    /// pressing the diamond meant.
    pub fn key_point(&self) -> Option<(&Clip, f64)> {
        let clip = self.sole_selection().and_then(|id| self.clip(&id))?;
        if clip.duration <= 0.0 {
            return None;
        }
        let local = f64::from(self.playhead) - clip.start;
        (0.0..=clip.duration)
            .contains(&local)
            .then(|| (clip, local / clip.duration))
    }

    /// The keyframe cluster's six rows, in the order the `Keyframes` global
    /// indexes them. Empty when nothing can be keyed, which is what greys
    /// every cluster in the inspector at once.
    pub fn key_rows(&self) -> Vec<ClipKeyData> {
        let Some((clip, at)) = self.key_point() else {
            return Vec::new();
        };
        model::KeyProperty::ALL
            .iter()
            .map(|&property| {
                let (prev, next) = clip.keys_around(property, at);
                ClipKeyData {
                    field: key_field_of(property),
                    keyed: clip.is_keyed(property),
                    here: clip.key_at(property, at).is_some(),
                    prev: prev.is_some(),
                    next: next.is_some(),
                }
            })
            .collect()
    }

    /// The Keyframes panel's rows: the six properties of the selected
    /// clip, and every Adjust knob of it that carries keys, each with its
    /// keys along the clip, what it is worth at the playhead, and the ease
    /// into the key the playhead is on or heading for.
    pub fn key_editor_rows(&self) -> Vec<KeyRowData> {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return Vec::new();
        };
        let at = place_in(clip, self.playhead);
        let near = |a: f64, b: f64| (a - b).abs() <= model::KEY_EPSILON;
        // The ease shown for a run of keys: the one into the key at the
        // playhead, else into the next key - the segment being played -
        // when there is a key behind it to ease from.
        let ease_of = |keys: &[(f64, model::KeyEase)]| -> (bool, [f32; 4], f32) {
            let target = keys
                .iter()
                .find(|(key_at, _)| near(*key_at, at))
                .or_else(|| keys.iter().find(|(key_at, _)| *key_at > at));
            match target {
                Some((key_at, ease))
                    if keys
                        .iter()
                        .any(|(other, _)| *other < key_at - model::KEY_EPSILON) =>
                {
                    let [x1, y1, x2, y2] = ease.0;
                    (
                        true,
                        [x1 as f32, y1 as f32, x2 as f32, y2 as f32],
                        *key_at as f32,
                    )
                }
                _ => (false, [0.0, 0.0, 1.0, 1.0], 0.0),
            }
        };
        let marks = |ats: &[f64]| -> ModelRc<KeyMarkData> {
            ModelRc::new(VecModel::from(
                ats.iter()
                    .map(|&key_at| KeyMarkData {
                        at: key_at as f32,
                        here: near(key_at, at),
                    })
                    .collect::<Vec<_>>(),
            ))
        };
        let mut rows = Vec::new();
        for property in model::KeyProperty::ALL {
            let keys: Vec<(f64, model::KeyEase)> = clip
                .keys_on(property)
                .map(|key| (key.at, key.ease))
                .collect();
            let ats: Vec<f64> = keys.iter().map(|(key_at, _)| *key_at).collect();
            // The same ranges and units as the inspector's knobs.
            let (label, minimum, maximum, step, default_value, fmt, unit, unit_scale) =
                match property {
                    model::KeyProperty::Scale => (
                        t("Scale"),
                        0.05,
                        8.0,
                        0.01,
                        1.0,
                        ParamFormat::Fraction,
                        "%",
                        100.0,
                    ),
                    model::KeyProperty::OffsetX => (
                        t("Position X"),
                        -1.0,
                        1.0,
                        0.005,
                        0.0,
                        ParamFormat::Centred,
                        "%",
                        100.0,
                    ),
                    model::KeyProperty::OffsetY => (
                        t("Position Y"),
                        -1.0,
                        1.0,
                        0.005,
                        0.0,
                        ParamFormat::Centred,
                        "%",
                        100.0,
                    ),
                    model::KeyProperty::Rotation => (
                        t("Rotation"),
                        -180.0,
                        180.0,
                        1.0,
                        0.0,
                        ParamFormat::Degrees,
                        "°",
                        1.0,
                    ),
                    model::KeyProperty::Opacity => (
                        t("Opacity"),
                        0.0,
                        1.0,
                        0.01,
                        1.0,
                        ParamFormat::Fraction,
                        "%",
                        100.0,
                    ),
                    model::KeyProperty::Volume => (
                        t("Level"),
                        -60.0,
                        24.0,
                        0.5,
                        0.0,
                        ParamFormat::Gain,
                        "",
                        1.0,
                    ),
                };
            let shown_value = shown(clip, property, at);
            let value = if property == model::KeyProperty::Volume {
                // The level knob speaks decibels, floored where silence is.
                if shown_value <= 0.001 {
                    -60.0
                } else {
                    (20.0 * shown_value.log10()).clamp(-60.0, 24.0)
                }
            } else {
                shown_value
            };
            let (prev, next) = clip.keys_around(property, at);
            let (has_ease, ease, ease_at) = ease_of(&keys);
            rows.push(KeyRowData {
                field: key_field_of(property),
                param: SharedString::default(),
                label: label.into(),
                keys: marks(&ats),
                state: ClipKeyData {
                    field: key_field_of(property),
                    keyed: !keys.is_empty(),
                    here: clip.key_at(property, at).is_some(),
                    prev: prev.is_some(),
                    next: next.is_some(),
                },
                value: value as f32,
                minimum,
                maximum,
                step,
                default_value,
                fmt,
                unit: unit.into(),
                unit_scale,
                ease_x1: ease[0],
                ease_y1: ease[1],
                ease_x2: ease[2],
                ease_y2: ease[3],
                has_ease,
                ease_at,
            });
        }
        if let Some(link) = clip
            .video_effects
            .iter()
            .find(|entry| entry.id == ADJUST_ID)
        {
            let inside = self.key_point().map(|(_, at)| at);
            for row in adjust_rows(&clip.video_effects, inside) {
                if !row.keyed {
                    continue;
                }
                let keys: Vec<(f64, model::KeyEase)> = link
                    .keys_on(&row.key)
                    .iter()
                    .map(|key| (key.at, key.ease))
                    .collect();
                let ats: Vec<f64> = keys.iter().map(|(key_at, _)| *key_at).collect();
                let (has_ease, ease, ease_at) = ease_of(&keys);
                rows.push(KeyRowData {
                    field: ClipField::Scale,
                    param: row.key.clone(),
                    label: row.label.clone(),
                    keys: marks(&ats),
                    state: ClipKeyData {
                        field: ClipField::Scale,
                        keyed: true,
                        here: row.here,
                        prev: row.prev,
                        next: row.next,
                    },
                    value: row.value,
                    minimum: row.min,
                    maximum: row.max,
                    step: row.step,
                    default_value: row.default_value,
                    fmt: row.fmt,
                    unit: row.unit.clone(),
                    unit_scale: 1.0,
                    ease_x1: ease[0],
                    ease_y1: ease[1],
                    ease_x2: ease[2],
                    ease_y2: ease[3],
                    has_ease,
                    ease_at,
                });
            }
        }
        rows
    }

    /// The selected clip's keys as instants on the timeline, every property
    /// and Adjust knob together, one diamond per instant: what the ruler
    /// draws while exactly one clip is selected (#188).
    pub fn key_marks(&self) -> Vec<f32> {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return Vec::new();
        };
        let mut ats: Vec<f64> = model::KeyProperty::ALL
            .iter()
            .flat_map(|&property| clip.keys_on(property).map(|key| key.at))
            .collect();
        if let Some(link) = clip
            .video_effects
            .iter()
            .find(|entry| entry.id == ADJUST_ID)
        {
            ats.extend(link.keys.values().flatten().map(|key| key.at));
        }
        ats.sort_by(f64::total_cmp);
        ats.dedup_by(|a, b| (*a - *b).abs() <= model::KEY_EPSILON);
        ats.iter()
            .map(|at| (clip.start + at * clip.duration) as f32)
            .collect()
    }

    /// Moves every key of the selected clip that sits at the instant
    /// `from` to the instant `to`, as one undo step: what a dragged ruler
    /// diamond does.
    pub fn move_keys_at(&mut self, from: f32, to: f32) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return;
        };
        if clip.duration <= 0.0 {
            return;
        }
        let clip_id = clip.id.clone();
        let from_at = (f64::from(from) - clip.start) / clip.duration;
        let to_at = ((f64::from(to) - clip.start) / clip.duration).clamp(0.0, 1.0);
        let mut commands = Vec::new();
        for property in model::KeyProperty::ALL {
            let Some(index) = clip.key_at(property, from_at) else {
                continue;
            };
            let key = &clip.keys[index];
            commands.push(Command::ClearClipKey {
                clip_id: clip_id.clone(),
                property,
                at: key.at,
            });
            commands.push(Command::SetClipKey {
                clip_id: clip_id.clone(),
                property,
                at: to_at,
                value: key.value,
                ease: key.ease,
            });
        }
        if let Some(entry) = clip
            .video_effects
            .iter()
            .position(|entry| entry.id == ADJUST_ID)
        {
            let link = &clip.video_effects[entry];
            for name in link.keys.keys() {
                let Some(index) = link.key_at(name, from_at) else {
                    continue;
                };
                let key = &link.keys_on(name)[index];
                commands.push(Command::ClearEffectKey {
                    clip_id: clip_id.clone(),
                    entry,
                    key: name.clone(),
                    at: key.at,
                });
                commands.push(Command::SetEffectKey {
                    clip_id: clip_id.clone(),
                    entry,
                    key: name.clone(),
                    at: to_at,
                    value: key.value,
                    ease: key.ease,
                });
            }
        }
        if !commands.is_empty() {
            self.apply(Command::Batch { commands });
        }
    }

    /// Moves the playhead to `at` of the selected clip, `0..1`.
    pub fn jump_to_key(&mut self, at: f32) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return;
        };
        let (start, duration) = (clip.start, clip.duration);
        self.seek((start + f64::from(at).clamp(0.0, 1.0) * duration) as f32);
    }

    /// Moves the key at `from` to `to` on a property, or on the Adjust
    /// knob `param` names: the same value and ease at the new place, as
    /// one undo step.
    pub fn move_key(&mut self, field: ClipField, param: &str, from: f32, to: f32) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return;
        };
        let clip_id = clip.id.clone();
        let (from, to) = (f64::from(from), f64::from(to).clamp(0.0, 1.0));
        let commands = if param.is_empty() {
            let Some(property) = Self::key_property_of(field) else {
                return;
            };
            let Some(index) = clip.key_at(property, from) else {
                return;
            };
            let key = &clip.keys[index];
            let (value, ease) = (key.value, key.ease);
            vec![
                Command::ClearClipKey {
                    clip_id: clip_id.clone(),
                    property,
                    at: from,
                },
                Command::SetClipKey {
                    clip_id,
                    property,
                    at: to,
                    value,
                    ease,
                },
            ]
        } else {
            let Some(entry) = clip
                .video_effects
                .iter()
                .position(|entry| entry.id == ADJUST_ID)
            else {
                return;
            };
            let link = &clip.video_effects[entry];
            let Some(index) = link.key_at(param, from) else {
                return;
            };
            let key = &link.keys_on(param)[index];
            let (value, ease) = (key.value, key.ease);
            vec![
                Command::ClearEffectKey {
                    clip_id: clip_id.clone(),
                    entry,
                    key: param.to_owned(),
                    at: from,
                },
                Command::SetEffectKey {
                    clip_id,
                    entry,
                    key: param.to_owned(),
                    at: to,
                    value,
                    ease,
                },
            ]
        };
        self.apply(Command::Batch { commands });
    }

    /// Sets the ease into the key at `at`. A drag of the curve editor is
    /// many of these, folded into one undo step.
    pub fn set_key_ease(&mut self, field: ClipField, param: &str, at: f32, ease: [f32; 4]) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return;
        };
        let clip_id = clip.id.clone();
        let at = f64::from(at);
        let ease = model::KeyEase(ease.map(f64::from)).sane();
        let command = if param.is_empty() {
            let Some(property) = Self::key_property_of(field) else {
                return;
            };
            let Some(index) = clip.key_at(property, at) else {
                return;
            };
            Command::SetClipKey {
                clip_id,
                property,
                at,
                value: clip.keys[index].value,
                ease,
            }
        } else {
            let Some(entry) = clip
                .video_effects
                .iter()
                .position(|entry| entry.id == ADJUST_ID)
            else {
                return;
            };
            let link = &clip.video_effects[entry];
            let Some(index) = link.key_at(param, at) else {
                return;
            };
            Command::SetEffectKey {
                clip_id,
                entry,
                key: param.to_owned(),
                at,
                value: link.keys_on(param)[index].value,
                ease,
            }
        };
        self.apply_within("ease", command);
    }

    /// Moves the playhead to the nearest key behind (-1) or ahead (+1) on
    /// any knob of the selected clip.
    pub fn step_any_key(&mut self, delta: i32) {
        let Some((clip, at)) = self.key_point() else {
            return;
        };
        let mut candidates: Vec<f64> = model::KeyProperty::ALL
            .iter()
            .flat_map(|&property| clip.keys_on(property).map(|key| key.at))
            .collect();
        if let Some(link) = clip
            .video_effects
            .iter()
            .find(|entry| entry.id == ADJUST_ID)
        {
            candidates.extend(link.keys.values().flatten().map(|key| key.at));
        }
        let target = if delta < 0 {
            candidates
                .iter()
                .copied()
                .filter(|&key_at| key_at < at - model::KEY_EPSILON)
                .reduce(f64::max)
        } else {
            candidates
                .iter()
                .copied()
                .filter(|&key_at| key_at > at + model::KEY_EPSILON)
                .reduce(f64::min)
        };
        let Some(target) = target else {
            return;
        };
        let (start, duration) = (clip.start, clip.duration);
        self.seek((start + target * duration) as f32);
    }

    /// Takes every key off the selected clip: its properties and its
    /// Adjust knobs, as one undo step.
    pub fn clear_all_keys(&mut self) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return;
        };
        let clip_id = clip.id.clone();
        let mut commands: Vec<Command> = model::KeyProperty::ALL
            .iter()
            .filter(|&&property| clip.is_keyed(property))
            .map(|&property| Command::ClearClipKeys {
                clip_id: clip_id.clone(),
                property,
            })
            .collect();
        if let Some(entry) = clip
            .video_effects
            .iter()
            .position(|entry| entry.id == ADJUST_ID)
        {
            for key in clip.video_effects[entry].keys.keys() {
                commands.push(Command::ClearEffectKeys {
                    clip_id: clip_id.clone(),
                    entry,
                    key: key.clone(),
                });
            }
        }
        if !commands.is_empty() {
            self.apply(Command::Batch { commands });
        }
    }

    /// Puts a key on the field at the playhead, or takes off the one there.
    ///
    /// The value a new key gets is whatever the property is worth at that
    /// instant - the constant on a clip with no keys, and the ride's own
    /// value on one that has them. That is what makes the diamond safe to
    /// press: it never moves the picture, it only says "hold this here".
    pub fn toggle_key(&mut self, field: ClipField) {
        let Some(property) = Self::key_property_of(field) else {
            return;
        };
        let Some((clip, at)) = self.key_point() else {
            return;
        };
        let clip_id = clip.id.clone();
        let command = if clip.key_at(property, at).is_some() {
            Command::ClearClipKey {
                clip_id,
                property,
                at,
            }
        } else {
            let value = clip.value_at(property, at);
            // The ease of whichever key this one is joining behind, so
            // laying a run of keys down does not alternate between shapes.
            // The first key on a property has nothing to inherit and gets
            // the straight line.
            let ease = clip
                .keys_on(property)
                .rfind(|key| key.at < at)
                .map_or(model::KeyEase::LINEAR, |key| key.ease);
            Command::SetClipKey {
                clip_id,
                property,
                at,
                value,
                ease,
            }
        };
        self.apply(command);
    }

    /// Takes every key off a field, leaving it its constant.
    pub fn clear_keys_on(&mut self, field: ClipField) {
        let Some(property) = Self::key_property_of(field) else {
            return;
        };
        let Some(clip_id) = self.sole_selection() else {
            return;
        };
        self.apply(Command::ClearClipKeys { clip_id, property });
    }

    /// Moves the playhead to this field's previous (-1) or next (+1) key.
    pub fn step_key(&mut self, field: ClipField, delta: i32) {
        let Some(property) = Self::key_property_of(field) else {
            return;
        };
        let Some((clip, at)) = self.key_point() else {
            return;
        };
        let (start, duration) = (clip.start, clip.duration);
        let (prev, next) = clip.keys_around(property, at);
        let Some(target) = (if delta < 0 { prev } else { next }) else {
            return;
        };
        self.seek((start + target * duration) as f32);
    }

    /// The audio tracks the Audio panel offers for the selection: one label
    /// per track of the selected clip's media when it has more than one,
    /// else nothing - a file with one track has nothing to choose.
    fn audio_track_labels(&self) -> Vec<SharedString> {
        let Some(item) = self
            .sole_selection()
            .and_then(|id| self.clip(&id))
            .and_then(|clip| self.project().media_by_id(&clip.media_id))
        else {
            return Vec::new();
        };
        if item.audio_tracks.len() < 2 {
            return Vec::new();
        }
        item.audio_tracks
            .iter()
            .enumerate()
            .map(|(position, track)| audio_track_label(track, position).into())
            .collect()
    }

    /// The selection, flattened for the inspector: exactly one clip or
    /// nothing.
    fn selected(&self) -> SelectedClipData {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return SelectedClipData::default();
        };
        // A keyed property shows what it is worth at the playhead, which is
        // what the knob edits.
        let at = place_in(clip, self.playhead);
        let text = clip.text.clone().unwrap_or_default();
        let fill = colour_of(&text.color);
        let stroke = colour_of(&text.stroke_color);
        let plate = colour_of(&text.background);
        let analysis = clip.cutout.as_ref().and_then(|cutout| {
            self.cutout_jobs
                .get(&Self::analysis_key(&clip.media_id, cutout.subject))
                .copied()
        });
        SelectedClipData {
            present: true,
            frame_width: self.output_size().0 as i32,
            frame_height: self.output_size().1 as i32,
            id: clip.id.as_str().into(),
            name: clip.name.as_str().into(),
            kind: kind_of(clip),
            duration: clip.duration as f32,
            transition_id: clip
                .transition_in
                .as_ref()
                .map(|transition| transition.id.as_str())
                .unwrap_or_default()
                .into(),
            transition_name: clip
                .transition_in
                .as_ref()
                .and_then(|transition| Catalogue::builtin().get(&transition.id))
                .map(|package| t(&package.manifest.effect.name))
                .unwrap_or_default()
                .into(),
            transition_duration: clip
                .transition_in
                .as_ref()
                .map(|transition| transition.duration as f32)
                .unwrap_or(0.5),
            scale: shown(clip, model::KeyProperty::Scale, at) as f32,
            offset_x: shown(clip, model::KeyProperty::OffsetX, at) as f32,
            offset_y: shown(clip, model::KeyProperty::OffsetY, at) as f32,
            rotation: shown(clip, model::KeyProperty::Rotation, at) as f32,
            stretch_x: clip.stretch_x as f32,
            stretch_y: clip.stretch_y as f32,
            opacity: shown(clip, model::KeyProperty::Opacity, at) as f32,
            volume: shown(clip, model::KeyProperty::Volume, at) as f32,
            audio_track: self
                .project()
                .media_by_id(&clip.media_id)
                .map_or(0, |item| item.audio_track_position(clip.audio_stream))
                as i32,
            speed: clip.speed as f32,
            preserve_pitch: clip.preserve_pitch,
            speed_curve: match &clip.speed_curve {
                None => -1,
                Some(points) => concat_project::speed::preset_of(points)
                    .map(|index| index as i32)
                    .unwrap_or(concat_project::speed::PRESETS.len() as i32),
            },
            reverse: clip.reverse,
            flip_h: clip.flip_h,
            flip_v: clip.flip_v,
            blend: concat_core::Blend::ALL
                .iter()
                .position(|mode| *mode == concat_core::Blend::parse(&clip.blend))
                .unwrap_or(0) as i32,
            crop_left: clip.crop.map(|crop| crop.left as f32).unwrap_or(0.0),
            crop_top: clip.crop.map(|crop| crop.top as f32).unwrap_or(0.0),
            crop_right: clip.crop.map(|crop| crop.right as f32).unwrap_or(0.0),
            crop_bottom: clip.crop.map(|crop| crop.bottom as f32).unwrap_or(0.0),
            fade_in: clip.fade_in as f32,
            fade_out: clip.fade_out as f32,
            content: text.content.as_str().into(),
            font_family: text.font_family.trim_matches('"').into(),
            font_size: text.font_size as f32,
            font_weight: text.font_weight as f32,
            italic: text.italic,
            fill,
            fill_hex: hex_of(fill).into(),
            align: align_of(text.align),
            text_opacity: text.opacity as f32,
            stroke_width: text.stroke_width as f32,
            stroke,
            stroke_hex: hex_of(stroke).into(),
            stroke_opacity: f32::from(stroke.alpha()) / 255.0,
            shadow: text.shadow,
            plate,
            plate_hex: hex_of(plate).into(),
            plated: plate.alpha() > 0,
            line_height: text.line_height as f32,
            tracking: text.tracking as f32,
            text_width: text.max_width as f32,
            text_height: text.max_height as f32,
            cutout: match &clip.cutout {
                None => 0,
                Some(cutout) if cutout.mode == model::CutoutMode::Auto => 1,
                Some(_) => 2,
            },
            cutout_feather: clip
                .cutout
                .as_ref()
                .map(|cutout| cutout.feather as f32)
                .unwrap_or(model::DEFAULT_FEATHER as f32),
            cutout_strokes: clip
                .cutout
                .as_ref()
                .map(|cutout| cutout.strokes.len() as i32)
                .unwrap_or(0),
            cutout_subject: clip
                .cutout
                .as_ref()
                .map(|cutout| match cutout.subject {
                    model::Subject::Auto => 0,
                    model::Subject::Person => 1,
                    model::Subject::Object => 2,
                })
                .unwrap_or(0),
            cutout_progress: analysis.map(|(_, fraction)| fraction).unwrap_or(-1.0),
            cutout_fetching: analysis.is_some_and(|(fetching, _)| fetching),
            cutout_empty: self.cutout_empty(clip),
            enhance_progress: self
                .enhance_jobs
                .get(&clip.id)
                .map(|(_, fraction)| *fraction)
                .unwrap_or(-1.0),
            enhance_fetching: self
                .enhance_jobs
                .get(&clip.id)
                .is_some_and(|(fetching, _)| *fetching),
        }
    }

    /// The source instant of `clip` under the playhead, held to the clip.
    fn source_at_playhead(&self, clip: &Clip) -> f64 {
        let along = (f64::from(self.playhead) - clip.start).clamp(0.0, clip.duration);
        clip.source_start + along * clip.speed
    }

    /// Whether the cutout model found nothing at the playhead's frame of
    /// `clip`, so the picture is showing as shot. Only an automatic cutout
    /// says so: a custom one is whatever was painted.
    fn cutout_empty(&self, clip: &Clip) -> bool {
        let Some(cutout) = clip.cutout.as_ref() else {
            return false;
        };
        if cutout.mode != model::CutoutMode::Auto {
            return false;
        }
        let (Some(session), Some(media)) = (
            self.session.as_ref(),
            self.project().media_by_id(&clip.media_id),
        ) else {
            return false;
        };
        let project = std::path::Path::new(session.path());
        let store = concat_vision::MaskStore::open(&concat_vision::mask_dir(
            project,
            &media.path,
            cutout.subject,
        ));
        store
            .mask_at(self.source_at_playhead(clip))
            .is_some_and(|mask| mask.is_blank())
    }

    /// Starts reading the first smart stroke whose region is not there
    /// yet, unless one is being read. Called after every change, and by
    /// the finished job for whatever is next.
    pub fn ensure_regions(&mut self) {
        if self.region_job.is_some() || self.host.brushes.is_busy() {
            return;
        }
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project = std::path::PathBuf::from(session.path());
        let mut next: Option<(String, RegionRequest)> = None;
        'clips: for clip in &self.timeline().clips {
            let Some(cutout) = clip.cutout.as_ref() else {
                continue;
            };
            if cutout.mode != model::CutoutMode::Custom || !clip.kind.is_visual() {
                continue;
            }
            let Some(media) = self.project().media_by_id(&clip.media_id) else {
                continue;
            };
            // The source the clip shows, as the mask analysis reckons it.
            let range = (
                clip.source_start,
                clip.source_start + clip.duration * clip.speed.max(0.0625),
            );
            for stroke in &cutout.strokes {
                let request = RegionRequest {
                    project: project.clone(),
                    media_path: media.path.clone(),
                    media_size: (media.width.unwrap_or(0), media.height.unwrap_or(0)),
                    still: media.kind == model::MediaKind::Image,
                    subject: cutout.subject,
                    ranges: vec![range],
                    stroke: stroke.clone(),
                };
                if request.outstanding() {
                    next = Some((Self::analysis_key(&media.id, cutout.subject), request));
                    break 'clips;
                }
            }
        }
        let Some((key, request)) = next else {
            return;
        };
        self.region_job = Some(key.clone());
        self.cutout_jobs.entry(key.clone()).or_insert((false, 0.0));
        let brushes = Arc::clone(&self.host.brushes);
        let epoch = crate::host::project_epoch();
        spawn_in_project(
            move || {
                let reporting = key.clone();
                let result = brushes.read(&request, &mut |progress| {
                    let now = match progress {
                        concat_host::cutout::Progress::Fetching { received, total } => {
                            (true, received as f32 / total.max(1) as f32)
                        }
                        concat_host::cutout::Progress::Analysing(fraction) => (false, fraction),
                    };
                    let key = reporting.clone();
                    on_ui_in_project(epoch, move |studio, _, _| {
                        if let Some(held) = studio.cutout_jobs.get_mut(&key) {
                            *held = now;
                        }
                    });
                });
                (key, result)
            },
            |studio, _, _, (key, result)| {
                studio.region_job = None;
                studio.cutout_jobs.remove(&key);
                studio.pending_stroke = None;
                match result {
                    Ok(()) => {
                        studio.request_preview();
                        studio.ensure_regions();
                    }
                    Err(error) if error.contains("cancelled") => {}
                    Err(error) => studio.notify(&tf("Smart brush: {0}", &[&error]), true),
                }
            },
        );
    }

    /// The menus, the dialogs, the bin and the engine lists.
    pub fn publish_chrome(&self, app: &App, models: &Models) {
        let editor = app.global::<Editor>();

        // The language, when it changed: one property the whole tree
        // reads, written only when it differs so nothing re-evaluates
        // for nothing.
        let words = app.global::<I18n>();
        let lang = SharedString::from(i18n::current());
        if words.get_lang() != lang {
            words.set_lang(lang);
        }

        // The catalogue's shelves. The groups are built in and never change;
        // the entries are whatever each library's search, shelf and star let
        // through, so they are rebuilt here and `sync` makes an unchanged
        // one a no-op.
        let starred = &self.prefs.favourites;
        let stamp = ShelfStamp {
            catalogue: std::ptr::from_ref(Catalogue::builtin()) as usize,
            lang: i18n::current(),
            views: self
                .library
                .iter()
                .map(|view| {
                    (
                        view.query.clone(),
                        view.group,
                        view.favourites,
                        view.category.clone(),
                    )
                })
                .collect(),
            favourites: starred.clone(),
        };
        if self.shelf_stamp.borrow().as_ref() != Some(&stamp) {
            let (groups, entries) =
                shelves(SHELF_KINDS[0], &self.library[0], starred, &self.look_art);
            sync(&models.filter_groups, groups);
            sync(&models.catalogue_filters, entries);
            let (groups, entries) =
                shelves(SHELF_KINDS[1], &self.library[1], starred, &self.look_art);
            sync(&models.effect_groups, groups);
            sync(&models.catalogue_effects, entries);
            let (groups, entries) =
                shelves(SHELF_KINDS[2], &self.library[2], starred, &self.look_art);
            sync(&models.audio_groups, groups);
            sync(&models.catalogue_audio, entries);
            let (groups, entries) =
                shelves(SHELF_KINDS[3], &self.library[3], starred, &self.look_art);
            sync(&models.transition_groups, groups);
            sync(&models.catalogue_transitions, entries);
            *self.shelf_stamp.borrow_mut() = Some(stamp);
        }
        sync(
            &models.library_views,
            self.library
                .iter()
                .map(|view| LibraryViewData {
                    query: view.query.as_str().into(),
                    group: view.group,
                    favourites: view.favourites,
                    category: view.category.as_str().into(),
                })
                .collect(),
        );

        app.set_on_start(self.on_start);
        app.set_project_name(self.project_name.as_str().into());
        app.set_project_status(
            if self.dirty {
                "unsaved changes"
            } else {
                "saved"
            }
            .into(),
        );
        app.set_toast(ToastData {
            token: self.toast.token,
            message: self.toast.message.as_str().into(),
            failed: self.toast.failed,
        });
        app.set_start(self.start.data());
        sync(
            &models.recents,
            self.recents
                .iter()
                .map(|project| RecentProjectData {
                    path: project.path.as_str().into(),
                    name: project.name.as_str().into(),
                    detail: format!(
                        "{} x {} · {:.2} fps",
                        project.width,
                        project.height,
                        project.rate_num as f32 / project.rate_den.max(1) as f32
                    )
                    .into(),
                    when: when_phrase(project.opened_at).into(),
                    poster: self.posters.get(&project.path).cloned().unwrap_or_default(),
                })
                .collect(),
        );

        // The Text page's presets: the look each card draws its name in.
        sync(
            &models.text_presets,
            self.text_presets
                .iter()
                .map(|preset| {
                    let plate = colour_of(&preset.style.background);
                    TextPresetData {
                        id: preset.id.as_str().into(),
                        name: t(&preset.name).into(),
                        family: preset.style.font_family.trim_matches('"').into(),
                        weight: preset.style.font_weight.round() as i32,
                        italic: preset.style.italic,
                        fill: colour_of(&preset.style.color),
                        plate,
                        plated: plate.alpha() > 0,
                        stroke: colour_of(&preset.style.stroke_color),
                        stroke_width: preset.style.stroke_width as f32,
                        align: align_of(preset.style.align),
                    }
                })
                .collect(),
        );

        // The bin.
        // The Media shelves count the imports; what the editor made is
        // counted on its own shelf under Generated. See MediaBin::shows.
        let items = &self.project().media;
        sync(&models.media, self.media.rows(self));
        let imported = |kind: Option<model::MediaKind>| {
            items
                .iter()
                .filter(|item| item.origin.is_none() && kind.is_none_or(|kind| item.kind == kind))
                .count() as i32
        };
        editor.set_media_count_all(imported(None));
        editor.set_media_count_video(imported(Some(model::MediaKind::Video)));
        editor.set_media_count_audio(imported(Some(model::MediaKind::Audio)));
        editor.set_media_count_images(imported(Some(model::MediaKind::Image)));
        editor.set_media_count_speech(
            items
                .iter()
                .filter(|item| item.origin == Some(model::MediaOrigin::Speech))
                .count() as i32,
        );
        editor.set_media_count_processed(
            items
                .iter()
                .filter(|item| item.origin == Some(model::MediaOrigin::Processed))
                .count() as i32,
        );
        editor.set_media_selected_count(self.media.selected.len() as i32);
        editor.set_importing(false);

        let (width, height) = self.output_size();
        editor.set_output_width(width as i32);
        editor.set_output_height(height as i32);
        editor.set_ratio_index(
            OUTPUTS
                .iter()
                .position(|size| *size == (width as i32, height as i32))
                .map_or(-1, |index| index as i32),
        );
        editor.set_quality_index(self.quality_of() as i32);

        // The Details panel, and the sheet its Modify button opens. The
        // frame, rate and duration are the active timeline's, and the panel
        // says so by name; the name and folder are the project's.
        let folder: SharedString = self
            .session
            .as_ref()
            .map(|session| session.path().to_owned())
            .unwrap_or_else(|| "—".to_owned())
            .into();
        let timeline_name: SharedString = self.timeline().name.as_str().into();
        editor.set_project_name(self.project_name.as_str().into());
        editor.set_project_folder(folder.clone());
        editor.set_timeline_name(timeline_name.clone());
        editor.set_project_output(format!("{width} × {height}").into());
        editor.set_project_rate(format!("{:.2} fps", self.frame_rate()).into());
        editor.set_project_duration(frames_timecode(self.duration(), self.frame_rate()).into());
        editor.set_count_media(self.project().media.len() as i32);
        editor.set_count_tracks(self.timeline().tracks.len() as i32);
        editor.set_count_clips(self.timeline().clips.len() as i32);
        app.set_project_sheet(self.project_sheet.data(self));

        let rows = self.menu();
        editor.set_menu_height(Self::menu_height(&rows));
        sync(&models.menu, rows);
        editor.set_menu_token(self.menu_token);

        app.set_export(self.export.data(self));
        app.set_settings(self.settings.data(self));
        sync(&models.transcribers, self.settings.transcriber_rows());
        sync(&models.voices, self.settings.voice_rows());
        app.set_relink(self.relink.data());

        // The speech sheets, and the lists they choose from.
        let transcribers = installed(&self.settings.transcribers);
        sync(
            &models.caption_models,
            transcribers
                .iter()
                .map(|model| SharedString::from(model.name.as_str()))
                .collect(),
        );
        app.set_captions(self.captions.data(self));
        let voices = installed(&self.settings.voices);
        sync(
            &models.speech_models,
            voices
                .iter()
                .map(|model| SharedString::from(model.name.as_str()))
                .collect(),
        );
        sync(
            &models.speech_model_details,
            self.speech.model_detail_rows(self),
        );
        sync(&models.speakers, self.speech.speaker_rows());
        sync(&models.speaker_details, self.speech.speaker_detail_rows());
        sync(&models.speech_samples, self.speech.sample_rows(self));
        sync(
            &models.speech_sample_waves,
            self.speech.sample_wave_rows(self),
        );
        sync(
            &models.speech_sample_details,
            self.speech.sample_detail_rows(self),
        );
        app.set_speech(self.speech.data(self));

        let bar = self.menu_bar();
        app.set_app_menu_height(Self::menu_height(&bar));
        sync(&models.bar, bar);
        app.set_app_menu_token(self.menu_bar_token);
        app.set_open_menu(self.open_menu);
    }

    /// Asks the workers for anything the launch screen or the bin is
    /// missing. Separate from `publish` because it mutates.
    pub fn refresh_art(&mut self) {
        // Called after every callback, the pointer stream included. The
        // scan over every media item - stats for proxies, JPEGs read back,
        // decodes queued - runs when the document, the screen or what is
        // pending changed, and is one comparison otherwise. The lanes'
        // cell strips follow the view, which moves on that stream, so
        // they are asked for every time; that walk is hash lookups
        // (audit 2026-09-23, #11).
        let stamp = (
            self.revision,
            self.on_start,
            self.art_pending.len() + self.posters_pending.len(),
        );
        if self.art_stamp == Some(stamp) {
            if !self.on_start {
                self.request_window_art();
            }
            return;
        }
        self.art_stamp = Some(stamp);
        if self.on_start {
            self.request_posters();
        } else {
            self.assign_media_rows();
            self.request_media_art();
        }
    }

    /// The right-click menu: the bin card's when it was opened on one,
    /// else the clip's.
    fn menu(&self) -> Vec<MenuItemData> {
        if let Some(id) = self.menu_media.as_deref() {
            return self.media_menu(id);
        }
        let Some(clip) = self.menu_target.as_ref().and_then(|id| self.clip(id)) else {
            return Vec::new();
        };
        let locked = self.locked(&clip.track_id);
        let playhead = f64::from(self.playhead);
        let straddled = clip.start < playhead && playhead < clip.start + clip.duration;

        let action =
            |id: &str, label: String, glyph: Glyph, shortcut: &str, enabled: bool| MenuItemData {
                id: id.into(),
                label: label.into(),
                kind: MenuRow::Action,
                glyph,
                shortcut: shortcut.into(),
                enabled,
                danger: false,
                checkable: false,
                checked: false,
            };
        let check = |id: &str, label: &str, shortcut: &str, on: bool, enabled: bool| MenuItemData {
            id: id.into(),
            label: label.into(),
            kind: MenuRow::Action,
            glyph: Glyph::None,
            shortcut: shortcut.into(),
            enabled,
            danger: false,
            checkable: true,
            checked: on,
        };
        let rule = || MenuItemData {
            kind: MenuRow::Separator,
            ..Default::default()
        };

        let mut rows = vec![
            action("copy", t("Copy"), Glyph::Copy, "⌘C", true),
            action("duplicate", t("Duplicate"), Glyph::Plus, "⌘D", !locked),
            action(
                "paste",
                t("Paste"),
                Glyph::Plus,
                "⌘V",
                self.clipboard.is_some(),
            ),
            rule(),
            action(
                "split",
                t("Split at playhead"),
                Glyph::Split,
                "S",
                straddled && !locked,
            ),
            action(
                "freeze",
                "Freeze frame".into(),
                Glyph::Frame,
                "F",
                straddled
                    && !locked
                    && (clip.kind == model::ClipKind::Video || clip.kind == model::ClipKind::Image),
            ),
            // The picture restored and enlarged by the model, as a copy
            // the clip then shows. Greyed while one is being written,
            // since the job runs one at a time.
            action(
                "enhance",
                t("Enhance"),
                Glyph::Sparkle,
                "",
                !locked
                    && (clip.kind == model::ClipKind::Video || clip.kind == model::ClipKind::Image)
                    && self.enhance_jobs.is_empty(),
            ),
            rule(),
        ];
        // One slot, two verbs: the sound is either on its picture or off it.
        // Shown on every video clip, so the verb is where a person looks for
        // it, and greyed rather than gone when the file has no sound.
        let (can_detach, can_reattach) = self.sound_placement(clip);
        if can_reattach {
            rows.push(action(
                "reattach",
                t("Reattach audio"),
                Glyph::Merge,
                "",
                !locked,
            ));
            rows.push(rule());
        } else if clip.kind == model::ClipKind::Video {
            rows.push(action(
                "detach",
                t("Detach audio"),
                Glyph::Waveform,
                "",
                !locked && can_detach && self.clip_has_sound(clip),
            ));
            rows.push(rule());
        }
        // The clip's sound, written out as it plays, as a file of its own
        // in the bin. For anything with sound in it; a clip whose track is
        // muted still renders, since the file is the clip's and not the
        // mix's.
        if clip.kind == model::ClipKind::Audio || clip.kind == model::ClipKind::Video {
            rows.push(action(
                "render-sound",
                t("Render the sound as a file"),
                Glyph::Waveform,
                "",
                self.clip_has_sound(clip),
            ));
            rows.push(rule());
        }
        let audible = clip.kind != model::ClipKind::Image;
        rows.push(check(
            "mute",
            &t("Mute"),
            "M",
            clip.volume <= 0.0,
            !locked && audible,
        ));
        rows.push(check("lock", &t("Lock track"), "", locked, true));
        rows.push(rule());
        rows.push(MenuItemData {
            id: "delete".into(),
            label: t("Delete").into(),
            kind: MenuRow::Action,
            glyph: Glyph::Trash,
            shortcut: "⌫".into(),
            enabled: !locked,
            danger: true,
            checkable: false,
            checked: false,
        });
        rows.push(MenuItemData {
            id: "ripple-delete".into(),
            label: t("Ripple delete").into(),
            kind: MenuRow::Action,
            glyph: Glyph::Trash,
            shortcut: "⇧⌫".into(),
            enabled: !locked,
            danger: true,
            checkable: false,
            checked: false,
        });
        rows
    }

    /// The right-click menu for a card in the bin: the file's verbs, and
    /// the one attribute a file has that every clip cut from it inherits.
    ///
    /// A picture's colour range is the media's rather than any clip's -
    /// the fix for a file that lies about its levels is a fact about the
    /// file - so it is set here, off the card, the way Resolve keeps Clip
    /// Attributes off the media pool, and not in a clip's inspector where
    /// it read as a property of the cut. Auto is the file's tag, and the
    /// codec's convention where there is none; see
    /// `concat_media::ColorRange::implied`.
    /// https://github.com/jub0t/Concat/issues/103
    fn media_menu(&self, id: &str) -> Vec<MenuItemData> {
        let Some(item) = self.project().media_by_id(id) else {
            return Vec::new();
        };
        let action = |id: &str, label: String, glyph: Glyph, danger: bool| MenuItemData {
            id: id.into(),
            label: label.into(),
            kind: MenuRow::Action,
            glyph,
            shortcut: "".into(),
            enabled: true,
            danger,
            checkable: false,
            checked: false,
        };
        // The levels go where a shortcut would: the number is what the
        // word means, and a person who knows one knows the other.
        let check = |id: &str, label: String, levels: &str, on: bool| MenuItemData {
            id: id.into(),
            label: label.into(),
            kind: MenuRow::Action,
            glyph: Glyph::None,
            shortcut: levels.into(),
            enabled: true,
            danger: false,
            checkable: true,
            checked: on,
        };
        let rule = || MenuItemData {
            kind: MenuRow::Separator,
            ..Default::default()
        };
        let mut rows = vec![
            action("add", t("Add at playhead"), Glyph::Plus, false),
            rule(),
        ];
        if item.kind != model::MediaKind::Audio {
            let range = item.color_range;
            rows.push(MenuItemData {
                label: t("Colour range").to_uppercase().into(),
                kind: MenuRow::Label,
                ..Default::default()
            });
            rows.push(check("range-auto", t("Auto"), "", range.is_none()));
            rows.push(check(
                "range-limited",
                t("Limited"),
                "16-235",
                range == Some(model::ColorRange::Limited),
            ));
            rows.push(check(
                "range-full",
                t("Full"),
                "0-255",
                range == Some(model::ColorRange::Full),
            ));
            rows.push(rule());
        }
        rows.push(action("remove", t("Remove"), Glyph::Trash, true));
        rows
    }

    /// A row of a bin card's menu, for the media it was opened on.
    pub fn media_action(&mut self, id: &str, action: &str) {
        let Some(row) = self.media.row_of(id) else {
            return;
        };
        if self.project().media_by_id(id).is_none() {
            return;
        }
        let range = |range| Command::SetMediaColorRange {
            media_id: id.to_owned(),
            range,
        };
        match action {
            "add" => self.place_at_playhead(&format!("media:{row}")),
            "remove" => self.handle(crate::panes::Msg::Media(
                crate::panes::media_bin::MediaMsg::Remove(row),
            )),
            "range-auto" => {
                self.apply(range(None));
            }
            "range-limited" => {
                self.apply(range(Some(model::ColorRange::Limited)));
            }
            "range-full" => {
                self.apply(range(Some(model::ColorRange::Full)));
            }
            _ => {}
        }
    }

    pub fn menu_height(rows: &[MenuItemData]) -> f32 {
        let metrics = |kind: MenuRow| match kind {
            MenuRow::Action => 26.0,
            MenuRow::Label => 24.0,
            MenuRow::Separator => 9.0,
        };
        rows.iter().map(|row| metrics(row.kind)).sum::<f32>() + 12.0
    }

    fn menu_bar(&self) -> Vec<MenuItemData> {
        let row =
            |id: &str, label: String, glyph: Glyph, shortcut: &str, enabled: bool| MenuItemData {
                id: id.into(),
                label: label.into(),
                kind: MenuRow::Action,
                glyph,
                shortcut: shortcut.into(),
                enabled,
                danger: false,
                checkable: false,
                checked: false,
            };
        let rule = || MenuItemData {
            kind: MenuRow::Separator,
            ..Default::default()
        };
        let check = |id: &str, label: &str, on: bool| MenuItemData {
            id: id.into(),
            label: label.into(),
            kind: MenuRow::Action,
            glyph: Glyph::None,
            shortcut: "".into(),
            enabled: true,
            danger: false,
            checkable: true,
            checked: on,
        };
        let selected = self.selection.len();
        let playhead = f64::from(self.playhead);
        let straddled = self.timeline().clips.iter().any(|clip| {
            clip.start + f64::from(MIN_DURATION) < playhead
                && playhead < clip.start + clip.duration - f64::from(MIN_DURATION)
        });
        let (can_undo, can_redo) = self.session.as_ref().map_or((false, false), |session| {
            (session.can_undo(), session.can_redo())
        });
        let has_selection_media = !self.media.selected.is_empty();

        match self.open_menu {
            0 => vec![
                row(
                    "add-selected",
                    t("Add selected to timeline"),
                    Glyph::Plus,
                    "",
                    has_selection_media,
                ),
                row("open", t("Open project…"), Glyph::Import, "⌘O", true),
                row("import", t("Import media…"), Glyph::Import, "⌘I", true),
                row("save", t("Save"), Glyph::Import, "⌘S", true),
                row(
                    "export",
                    t("Export…"),
                    Glyph::Export,
                    "",
                    !self.timeline().clips.is_empty(),
                ),
                row("template", t("Save as template…"), Glyph::Slot, "", true),
                row("speech", t("Text to speech…"), Glyph::Volume, "", true),
                row(
                    "clear-cache",
                    t("Clear project cache"),
                    Glyph::None,
                    "",
                    true,
                ),
                rule(),
                row("settings", t("Settings…"), Glyph::Settings, "⌘,", true),
                rule(),
                row("close-project", t("Close project"), Glyph::Import, "", true),
                MenuItemData {
                    id: "close-window".into(),
                    label: t("Close window").into(),
                    kind: MenuRow::Action,
                    glyph: Glyph::Close,
                    shortcut: "⌘W".into(),
                    enabled: true,
                    danger: true,
                    checkable: false,
                    checked: false,
                },
            ],
            1 => vec![
                row("undo", t("Undo"), Glyph::Undo, "⌘Z", can_undo),
                row("redo", t("Redo"), Glyph::Redo, "⇧⌘Z", can_redo),
                rule(),
                row(
                    "split",
                    t("Split at playhead"),
                    Glyph::Razor,
                    "⌘B",
                    straddled,
                ),
                MenuItemData {
                    id: "delete".into(),
                    label: if selected > 1 {
                        tf("Delete {0} clips", &[&selected])
                    } else {
                        t("Delete clip")
                    }
                    .into(),
                    kind: MenuRow::Action,
                    glyph: Glyph::Trash,
                    shortcut: "⌫".into(),
                    enabled: selected > 0,
                    danger: true,
                    checkable: false,
                    checked: false,
                },
                MenuItemData {
                    id: "ripple-delete".into(),
                    label: if selected > 1 {
                        tf("Ripple delete {0} clips", &[&selected])
                    } else {
                        t("Ripple delete")
                    }
                    .into(),
                    kind: MenuRow::Action,
                    glyph: Glyph::Trash,
                    shortcut: "⇧⌫".into(),
                    enabled: selected > 0,
                    danger: true,
                    checkable: false,
                    checked: false,
                },
                rule(),
                MenuItemData {
                    id: "snap".into(),
                    label: t("Snap to edges").into(),
                    kind: MenuRow::Action,
                    glyph: Glyph::None,
                    shortcut: "N".into(),
                    enabled: true,
                    danger: false,
                    checkable: true,
                    checked: self.lanes.snap,
                },
                MenuItemData {
                    id: "magnetic".into(),
                    label: t("Magnetic timeline").into(),
                    kind: MenuRow::Action,
                    glyph: Glyph::None,
                    shortcut: "".into(),
                    enabled: true,
                    danger: false,
                    checkable: true,
                    checked: self.prefs.magnetic,
                },
            ],
            2 => vec![
                row("zoom-in", t("Zoom in"), Glyph::Plus, "+", true),
                row("zoom-out", t("Zoom out"), Glyph::Minus, "-", true),
                rule(),
                check("sort-added", "Sort by: Added", self.media.sort == 0),
                check("sort-name", "Sort by: Name", self.media.sort == 1),
                check("sort-kind", "Sort by: Type", self.media.sort == 2),
                rule(),
                row("start", t("Go to start"), Glyph::SkipBack, "Home", true),
                row("end", t("Go to end"), Glyph::SkipForward, "End", true),
            ],
            _ => Vec::new(),
        }
    }

    /// The clip's track flags, as one command per flag that changed, plus
    /// the lock, which is the window's.
    pub fn track_flags(&mut self, row: i32, visible: bool, muted: bool, locked: bool) {
        let Some(track) = self.row_track(row).cloned() else {
            return;
        };
        let mut commands = Vec::new();
        if track.visible != visible {
            commands.push(Command::SetTrackFlag {
                track_id: track.id.clone(),
                flag: TrackFlag::Visible,
                value: visible,
            });
        }
        if track.muted != muted {
            commands.push(Command::SetTrackFlag {
                track_id: track.id.clone(),
                flag: TrackFlag::Muted,
                value: muted,
            });
        }
        let view = self.lanes.lane_view.entry(track.id.clone()).or_default();
        view.locked = locked;
        if locked {
            let doomed: Vec<String> = self
                .timeline()
                .clips
                .iter()
                .filter(|clip| clip.track_id == track.id)
                .map(|clip| clip.id.clone())
                .collect();
            self.selection.retain(|held| !doomed.contains(held));
        }
        match commands.len() {
            0 => {}
            1 => {
                self.apply(commands.remove(0));
            }
            _ => {
                self.apply(Command::Batch { commands });
            }
        }
    }

    pub fn set_lane_size(&mut self, row: i32, size: TrackSize) {
        if let Some(id) = self.row_track(row).map(|track| track.id.clone()) {
            self.lanes.lane_view.entry(id).or_default().size = size;
        }
    }

    pub fn toggle_flip_h(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let commands: Vec<Command> = self
            .selection
            .iter()
            .filter_map(|id| {
                self.clip(id).map(|clip| Command::UpdateClip {
                    clip_id: id.clone(),
                    patch: ClipPatch {
                        flip_h: Some(!clip.flip_h),
                        ..Default::default()
                    },
                })
            })
            .collect();
        if !commands.is_empty() {
            self.apply(Command::Batch { commands });
        }
    }

    pub fn toggle_flip_v(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let commands: Vec<Command> = self
            .selection
            .iter()
            .filter_map(|id| {
                self.clip(id).map(|clip| Command::UpdateClip {
                    clip_id: id.clone(),
                    patch: ClipPatch {
                        flip_v: Some(!clip.flip_v),
                        ..Default::default()
                    },
                })
            })
            .collect();
        if !commands.is_empty() {
            self.apply(Command::Batch { commands });
        }
    }

    pub fn toggle_reverse(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let commands: Vec<Command> = self
            .selection
            .iter()
            .filter_map(|id| {
                self.clip(id).map(|clip| Command::UpdateClip {
                    clip_id: id.clone(),
                    patch: ClipPatch {
                        reverse: Some(!clip.reverse),
                        ..Default::default()
                    },
                })
            })
            .collect();
        if !commands.is_empty() {
            self.apply(Command::Batch { commands });
        }
    }

    pub fn toggle_lock(&mut self, track_id: &str) {
        let view = self.lanes.lane_view.entry(track_id.to_owned()).or_default();
        view.locked = !view.locked;
        if view.locked {
            let doomed: Vec<String> = self
                .timeline()
                .clips
                .iter()
                .filter(|clip| clip.track_id == track_id)
                .map(|clip| clip.id.clone())
                .collect();
            self.selection.retain(|held| !doomed.contains(held));
        }
    }

    /// Changes the active timeline's output size from the monitor's picker.
    /// An edit like any other - undoable, and this timeline's alone.
    pub fn set_output(&mut self, index: usize) {
        let (width, height) = OUTPUTS[index.min(OUTPUTS.len() - 1)];
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let video = model::VideoSettings {
            width: width as u32,
            height: height as u32,
            ..session.video()
        };
        let timeline_id = self.project().active_timeline_id.clone();
        self.apply(Command::SetTimelineVideo { timeline_id, video });
        self.request_preview();
    }

    // ── the keyboard, and the menu's verbs ──

    /// A chord from the window's key table; see `Editor.shortcut`. The view
    /// chords - zoom, the playhead's ends, the sheets - go through the app
    /// menu's own handler in lib.rs, so a key and the row that advertises
    /// it are one thing.
    pub fn shortcut(&mut self, action: &str) {
        if !self.prefs.custom_context_actions && matches!(action, "flip-h" | "flip-v" | "reverse") {
            return;
        }
        match action {
            "split" => {
                let at = self.playhead;
                self.split_at(at, false);
            }
            "split-selected" => {
                let at = self.playhead;
                self.split_at(at, true);
            }
            "select-all" => self.select_all(),
            // ⇧⌫, from the key table; plain ⌫ comes in as its own callback.
            "ripple-delete" => self.ripple_delete_selected(),
            "copy" | "duplicate" | "mute" => {
                if let Some(id) = self.sole_selection() {
                    self.clip_action(&id, action);
                }
            }
            "flip-h" => self.toggle_flip_h(),
            "flip-v" => self.toggle_flip_v(),
            "reverse" => self.toggle_reverse(),
            "paste" => {
                let Some(held) = self.clipboard.clone() else {
                    return;
                };
                match self.sole_selection() {
                    // After the selected clip, on its lane, as the menu does.
                    Some(id) => self.clip_action(&id, "paste"),
                    // Nothing selected: at the playhead, on the lane it was
                    // copied from. `duplicate` lays a copy after its source,
                    // so the source is placed one length before the playhead.
                    None => {
                        let mut source = held;
                        source.start = f64::from(self.playhead) - source.duration;
                        self.duplicate(&source);
                    }
                }
            }
            _ => {}
        }
    }

    /// One of the clip menu's verbs on clip `id`. The menu's rows and the
    /// keyboard's chords both land here, so a shortcut and the row that
    /// advertises it cannot disagree.
    pub fn clip_action(&mut self, id: &str, action: &str) {
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        match action {
            "copy" => self.clipboard = Some(clip),
            "duplicate" => self.duplicate_selected(),
            "paste" => {
                if let Some(held) = self.clipboard.clone() {
                    let mut source = held;
                    // Pasted after the clip that was right-clicked, on its lane.
                    source.track_id = clip.track_id.clone();
                    source.start = clip.start + clip.duration - source.duration;
                    self.duplicate(&source);
                }
            }
            "split" => {
                let at = self.playhead;
                self.flush_commit();
                self.selection = vec![id.to_owned()];
                self.split_at(at, true);
            }
            "freeze" => self.freeze_at_playhead(),
            "enhance" => self.enhance_clip(id),
            "render-sound" => self.render_clip_sound(id),
            "detach" => {
                self.apply(Command::DetachAudio {
                    clip_id: id.to_owned(),
                });
            }
            "reattach" => {
                self.apply(Command::ReattachAudio {
                    clip_id: id.to_owned(),
                });
            }
            "mute" => {
                let volume = if clip.volume <= 0.0 { 1.0 } else { 0.0 };
                self.apply(Command::UpdateClip {
                    clip_id: id.to_owned(),
                    patch: ClipPatch {
                        volume: Some(volume),
                        ..Default::default()
                    },
                });
            }
            "lock" => self.toggle_lock(&clip.track_id),
            // A clip that is part of the selection takes the selection with
            // it: Delete on one of five selected clips means the five.
            "delete" | "ripple-delete" => {
                let ripple = action == "ripple-delete" || self.prefs.magnetic;
                if self.selection.len() > 1 && self.selection.iter().any(|held| held == id) {
                    self.remove_selected(ripple);
                } else {
                    self.apply(Command::RemoveClips {
                        clip_ids: vec![id.to_owned()],
                        ripple,
                    });
                }
                self.menu_target = None;
            }
            _ => {}
        }
    }

    /// Every clip on an unlocked lane.
    pub fn select_all(&mut self) {
        self.flush_commit();
        self.selection = self
            .timeline()
            .clips
            .iter()
            .filter(|clip| !self.locked(&clip.track_id))
            .map(|clip| clip.id.clone())
            .collect();
    }

    /// Tab `from` dropped at `slot`, a position counted over the strip as
    /// it stands; the command counts with the tab already removed.
    pub fn move_timeline(&mut self, from: i32, slot: i32) {
        let Some(from) = usize::try_from(from).ok() else {
            return;
        };
        let Some(timeline) = self.project().timelines.get(from) else {
            return;
        };
        let id = timeline.id.clone();
        let slot = slot.max(0) as usize;
        let index = if from < slot { slot - 1 } else { slot };
        if index == from {
            return;
        }
        self.apply(Command::MoveTimeline {
            timeline_id: id,
            index,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, Footprint, Studio, key_commands, place_in, shown, write_keyable};

    const FRAME: (u32, u32) = (1920, 1080);

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

    /// A knob over a property with no keys writes the constant; over one
    /// that rides, it writes the key at the playhead and leaves the
    /// constant alone. The echo's keys then become one command each.
    #[test]
    fn a_keyed_property_is_written_as_a_key_and_committed_as_commands() {
        use concat_project::model::{Clip, ClipKind, KeyEase, KeyProperty};
        let before = Clip::blank("c1", "t1", ClipKind::Video, "clip", 0.0, 10.0);
        let mut after = before.clone();
        write_keyable(&mut after, KeyProperty::Scale, 2.0, 0.5);
        assert_eq!(after.scale, 2.0, "no keys: the constant");
        assert!(key_commands("c1", &before, &after).is_empty());

        after.set_key(KeyProperty::Scale, 0.25, 1.0, KeyEase::LINEAR);
        write_keyable(&mut after, KeyProperty::Scale, 3.0, 0.75);
        assert_eq!(after.scale, 2.0, "keyed: the constant is left alone");
        assert_eq!(after.value_at(KeyProperty::Scale, 0.75), 3.0);
        assert_eq!(shown(&after, KeyProperty::Scale, 0.75), 3.0);
        let commands = key_commands("c1", &before, &after);
        assert_eq!(commands.len(), 2, "one per key: {commands:?}");
        assert!(commands.iter().all(|command| matches!(
            command,
            Command::SetClipKey {
                property: KeyProperty::Scale,
                ..
            }
        )));

        let mut gone = after.clone();
        gone.clear_key(KeyProperty::Scale, 0.25);
        let commands = key_commands("c1", &after, &gone);
        assert!(
            matches!(commands.as_slice(), [Command::ClearClipKey { at, .. }] if (*at - 0.25).abs() < 1e-9),
            "{commands:?}"
        );
        assert_eq!(place_in(&after, 2.5), 0.25);
        assert_eq!(place_in(&after, 12.0), 1.0, "held to the end");
    }
}

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

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use concat_effects::Catalogue;
use concat_effects::manifest::Kind as PackageKind;
use concat_host::export::{self, ExportSpec};
use concat_host::playback::ClipSpec;
use concat_host::preview::FrameSpec;
use concat_host::{ProjectInfo, Session, media, projects, templates};
use concat_media::Peaks;
use concat_project::commands::{ClipMove, ClipPatch, TrackFlag, TrimEdge};
use concat_project::model::{
    self, AppliedFilter, Clip, Project, TextAlign, TextStyle, Timeline, Track, Transition,
};
use concat_project::{Command, why_not_merge};
use slint::{Model, SharedString, VecModel};

use crate::dock::{
    Dock, DockLayout, SEAT_GAP, default_dock, lay_out, nearest_row, row_at, row_top,
};
use crate::format::{bytes, colour_of, eta, hex_of, hex_with_alpha, wave_path, when_phrase};
use crate::host::{Host, MediaArt, image_at, image_of, media_art, on_ui, spawn};
use crate::prefs::Preferences;
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
/// is what `TrackSize::Auto` picks from - see `lane_height`.
const LANE_LARGE: f32 = 80.0;
const LANE_MEDIUM: f32 = 60.0;
const LANE_SMALL: f32 = 40.0;

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
/// Megabits per second at 1080p30 for each quality tier, for the size
/// estimate; and the CRF each tier renders at.
const EXPORT_TIERS: [f32; 3] = [16.0, 8.0, 4.0];
const EXPORT_CRF: [u8; 3] = [16, 20, 26];
const AUDIO_BPS: f32 = 192_000.0;

/// The frame sizes the launch screen offers, and what each label means.
pub const RESOLUTIONS: [(&str, u32, u32); 4] = [
    ("1080p", 1920, 1080),
    ("720p", 1280, 720),
    ("4K", 3840, 2160),
    ("Vertical", 1080, 1920),
];

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

/// The interface's languages, as Settings > General lists them. One, until
/// the `.slint` tree's strings are wrapped for translation.
pub const LANGUAGES: [&str; 1] = ["English"];

/// The transcriber's languages: the rows of the Auto / English / Chinese
/// control in Settings > Transcriber, and the whisper code each means.
pub const TRANSCRIBE_LANGUAGES: [&str; 3] = ["auto", "en", "zh"];

/// The default Kokoro speaker: `af_heart`.
const DEFAULT_VOICE: i32 = 3;

/// The export sheet's state.
pub struct ExportState {
    pub open: bool,
    pub name: String,
    pub folder: String,
    pub resolution: usize,
    pub rate: usize,
    pub quality: usize,
    pub phase: ExportPhase,
    pub progress: f32,
    pub stage: String,
    pub message: String,
    /// Where the finished file is, for Reveal.
    pub written: String,
}

impl Default for ExportState {
    fn default() -> Self {
        Self {
            open: false,
            name: "Untitled".into(),
            folder: std::env::var("HOME")
                .map(|home| format!("{home}/Movies"))
                .unwrap_or_default(),
            resolution: 2,
            rate: 1,
            quality: 1,
            phase: ExportPhase::Idle,
            progress: 0.0,
            stage: String::new(),
            message: String::new(),
            written: String::new(),
        }
    }
}

/// The settings sheet's state.
#[derive(Default)]
pub struct SettingsState {
    pub open: bool,
    pub tab: i32,
    pub language: usize,
    pub transcribe_language: i32,
}

/// The bottom-right notice: one at a time. The token is what the panel
/// watches, bumped per notice so the same sentence said twice blinks twice.
#[derive(Default)]
pub struct ToastState {
    pub token: i32,
    pub message: String,
    pub failed: bool,
}

/// One downloadable model, as the settings sheet shows it.
#[derive(Clone)]
pub struct ModelState {
    pub id: String,
    pub name: String,
    pub note: String,
    pub megabytes: f32,
    pub accuracy: i32,
    pub installed: bool,
    pub active: bool,
    /// Megabytes fetched so far while a download runs.
    pub fetched: Option<f32>,
    pub unpacking: bool,
}

/// The form on the launch screen.
pub struct StartState {
    pub name: String,
    pub location: String,
    pub resolution: usize,
    pub rate: usize,
    pub busy: bool,
    pub error: String,
}

impl Default for StartState {
    fn default() -> Self {
        Self {
            name: "Untitled project".into(),
            location: std::env::var("HOME")
                .map(|home| format!("{home}/Desktop/Concat"))
                .unwrap_or_default(),
            resolution: 0,
            rate: 3,
            busy: false,
            error: String::new(),
        }
    }
}

/// What the window knows about a lane that the document does not: whether
/// it is locked, and how tall to draw it.
#[derive(Clone, Copy)]
pub struct LaneView {
    pub locked: bool,
    pub size: TrackSize,
}

impl Default for LaneView {
    fn default() -> Self {
        Self {
            locked: false,
            size: TrackSize::Auto,
        }
    }
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
    },
    /// The rotation grip dragged: the picture turns by the angle the pointer
    /// has swept about the centre since the press.
    StageRotate {
        clip: String,
        rotation: f64,
        centre: (f64, f64),
        from: f64,
    },
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
    pub catalogue_audio: Rc<VecModel<CatalogueEntryData>>,
    pub effect_groups: Rc<VecModel<SharedString>>,
    pub filter_groups: Rc<VecModel<SharedString>>,
    pub audio_groups: Rc<VecModel<SharedString>>,
    /// The selected clip's two chains and their knobs.
    pub applied_visual: Rc<VecModel<AppliedEntryData>>,
    pub applied_audio: Rc<VecModel<AppliedEntryData>>,
    pub visual_params: Rc<VecModel<AppliedParamData>>,
    pub audio_params: Rc<VecModel<AppliedParamData>>,
    /// The colour panel's knobs.
    pub adjust_params: Rc<VecModel<AppliedParamData>>,
    pub menu: Rc<VecModel<MenuItemData>>,
    pub av: Rc<VecModel<MenuItemData>>,
    pub bar: Rc<VecModel<MenuItemData>>,
    pub transcribers: Rc<VecModel<ModelData>>,
    pub voices: Rc<VecModel<ModelData>>,
    pub seats: Rc<VecModel<SeatBox>>,
    pub dividers: Rc<VecModel<DockDivider>>,
    pub recents: Rc<VecModel<RecentProjectData>>,
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
            effect_groups: Rc::new(VecModel::default()),
            filter_groups: Rc::new(VecModel::default()),
            audio_groups: Rc::new(VecModel::default()),
            applied_visual: Rc::new(VecModel::default()),
            applied_audio: Rc::new(VecModel::default()),
            visual_params: Rc::new(VecModel::default()),
            audio_params: Rc::new(VecModel::default()),
            adjust_params: Rc::new(VecModel::default()),
            menu: Rc::new(VecModel::default()),
            av: Rc::new(VecModel::default()),
            bar: Rc::new(VecModel::default()),
            transcribers: Rc::new(VecModel::default()),
            voices: Rc::new(VecModel::default()),
            seats: Rc::new(VecModel::default()),
            dividers: Rc::new(VecModel::default()),
            recents: Rc::new(VecModel::default()),
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
    /// Slint's rows are integers; the document's ids are strings. Assigned
    /// once per id and never reused, so a payload in flight names the row it
    /// was dragged from.
    media_rows: HashMap<String, i32>,
    next_media_row: i32,
    media_selected: HashSet<String>,
    media_filter: MediaFilter,
    /// Decoded art by media id, and the ids a worker is decoding for.
    pub peaks: HashMap<String, Arc<Peaks>>,
    pub thumbs: HashMap<String, slint::Image>,
    /// Filmstrips by media id: the picture, how many frames are in it, one
    /// frame's width and the strip's height, in the picture's own pixels.
    pub strips: HashMap<String, Strip>,
    art_pending: HashSet<String>,
    /// Envelopes, keyed by the things they are computed from. A move
    /// changes none of them, and a publish happens on every frame of one.
    waves: RefCell<HashMap<String, SharedString>>,

    // ── the view ──
    pub lane_view: HashMap<String, LaneView>,
    pub selection: Vec<String>,
    pub playhead: f32,
    pub scroll_left: f32,
    pub seconds_per_pixel: f32,
    pub tool: TimelineTool,
    pub snap: bool,
    /// 0 Full, 1 Half, 2 Quarter of the output size, for the monitor.
    pub quality: usize,
    pub playing: bool,
    transport: slint::Timer,
    /// One clip, held for Paste.
    pub clipboard: Option<Clip>,
    /// The monitor's last frame, and whether another is wanted.
    pub preview: slint::Image,
    preview_busy: bool,
    preview_wanted: bool,
    /// Said once per session: a monitor that cannot decode says so, and
    /// then stops repeating itself.
    preview_failed: bool,

    // ── the sheets and menus ──
    pub export: ExportState,
    pub settings: SettingsState,
    pub transcribers: Vec<ModelState>,
    pub voices: Vec<ModelState>,
    pub av_token: i32,
    pub open_menu: i32,
    pub menu_bar_token: i32,
    pub menu_target: Option<String>,
    pub menu_token: i32,
    pub toast: ToastState,

    // ── the launch screen ──
    pub on_start: bool,
    pub start: StartState,
    pub recents: Vec<ProjectInfo>,
    pub posters: HashMap<String, slint::Image>,
    posters_pending: HashSet<String>,
    pub project_name: String,

    // ── the workspace ──
    pub dock: Dock,
    pub workspace: (f32, f32),
    pub divider_press: Option<(usize, f32, f32)>,
    pub gesture: Gesture,
    /// The snap lines of a stage move in flight; empty between moves.
    pub stage_guides: Vec<StageGuideData>,
    /// Where the inspector should go, and a count that changes every time
    /// something is applied from the library; see `Editor.inspector-jump-token`.
    pub inspector_jump: (i32, &'static str, &'static str),
    /// The last inspector commit: what it changed and when. A control that
    /// is dragged commits on every move, and each of those would be an undo
    /// step of its own; a commit that changes the same thing as the last
    /// one, within a moment of it, replaces it in the history instead.
    last_commit: Option<(String, std::time::Instant)>,
    /// Each title's painted block in frame pixels, by clip id, as of the
    /// last time the monitor asked for a frame. What the stage box for a
    /// text clip is drawn from; see `footprint`.
    pub title_blocks: HashMap<String, (u32, u32)>,
    pub drop: Option<DropPlan>,
}

// ── conversions between the document and the window ─────────────────────

fn kind_of(clip: &Clip) -> ClipKind {
    match clip.kind {
        model::ClipKind::Video => ClipKind::Video,
        model::ClipKind::Audio => ClipKind::Audio,
        model::ClipKind::Image => ClipKind::Image,
        model::ClipKind::Text => ClipKind::Text,
        model::ClipKind::Layer => ClipKind::Filter,
    }
}

fn media_kind_of(kind: model::MediaKind) -> MediaKind {
    match kind {
        model::MediaKind::Video => MediaKind::Video,
        model::MediaKind::Audio => MediaKind::Audio,
        model::MediaKind::Image => MediaKind::Image,
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
        font_family: "Inter".to_owned(),
        font_weight: 600.0,
        ..TextStyle::default()
    }
}

/// A label for a catalogue id: "black-white" reads as "Black White".
/// One kind's shelves for the inspector: the shelf labels, in catalogue
/// order, and every package on them with the index of its shelf.
fn shelves(kind: PackageKind) -> (Vec<SharedString>, Vec<CatalogueEntryData>) {
    let mut groups: Vec<String> = Vec::new();
    let mut entries = Vec::new();
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
        let group = match groups.iter().position(|held| *held == category) {
            Some(index) => index,
            None => {
                groups.push(category.clone());
                groups.len() - 1
            }
        };
        entries.push(CatalogueEntryData {
            id: meta.id.as_str().into(),
            name: meta.name.as_str().into(),
            category: category.as_str().into(),
            group: group as i32,
            description: meta.description.as_str().into(),
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
            Command::SetClipSpeed { .. } => "speed".to_owned(),
            Command::SetClipSpeedCurve { .. } => "curve".to_owned(),
            Command::SetClipAnimation { slot, .. } => format!("animation:{slot:?}"),
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

/// The menu row of a slot's animation: -1 for none.
fn slot_index(
    slot: concat_project::model::AnimationSlot,
    set: &Option<concat_project::model::ClipAnimation>,
) -> i32 {
    set.as_ref()
        .and_then(|set| concat_project::animation::index_of(slot, &set.preset))
        .map(|index| index as i32)
        .unwrap_or(-1)
}

/// A slot set from its menu row: -1 takes it off, else the row's shape at
/// the length already set or `seconds`.
fn set_slot(
    slot: concat_project::model::AnimationSlot,
    field: &mut Option<concat_project::model::ClipAnimation>,
    row: f64,
    seconds: f64,
) {
    if row < 0.0 {
        *field = None;
        return;
    }
    let names = concat_project::animation::names(slot);
    let Some(name) = names.get(row as usize) else {
        return;
    };
    let duration = field.as_ref().map(|set| set.duration).unwrap_or(seconds);
    *field = Some(concat_project::model::ClipAnimation {
        preset: (*name).to_owned(),
        duration,
    });
}

/// The built-in colour package's id; see `adjust_rows` and `Studio::adjust_set`.
const ADJUST_ID: &str = "concat.adjust";

/// The colour panel's rows: the adjust package's parameters, at the values
/// the clip's chain holds or at the defaults when the clip carries none.
fn adjust_rows(chain: &[AppliedFilter]) -> Vec<AppliedParamData> {
    let Some(package) = Catalogue::builtin().get(ADJUST_ID) else {
        return Vec::new();
    };
    let held = chain.iter().find(|entry| entry.id == ADJUST_ID);
    package
        .manifest
        .params
        .iter()
        .map(|param| AppliedParamData {
            entry: -1,
            key: param.key.as_str().into(),
            label: param.label.as_str().into(),
            min: param.min as f32,
            max: param.max as f32,
            step: if param.step > 0.0 {
                param.step as f32
            } else {
                ((param.max - param.min) / 200.0) as f32
            },
            default_value: param.default as f32,
            value: held
                .and_then(|entry| entry.params.get(&param.key).copied())
                .unwrap_or(param.default) as f32,
            fmt: format_of(&param.unit),
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
                label: "Intensity".into(),
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
                label: param.label.as_str().into(),
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
        let recents = projects::list(&host.dirs.config);
        let mut studio = Self {
            prefs,
            session: None,
            echo: None,
            empty: Project::new(),
            dirty: false,
            autosave: slint::Timer::default(),
            media_rows: HashMap::new(),
            next_media_row: 1,
            media_selected: HashSet::new(),
            media_filter: MediaFilter::All,
            peaks: HashMap::new(),
            thumbs: HashMap::new(),
            strips: HashMap::new(),
            art_pending: HashSet::new(),
            waves: RefCell::new(HashMap::new()),
            lane_view: HashMap::new(),
            selection: Vec::new(),
            playhead: 0.0,
            scroll_left: 0.0,
            // ~20px a second: a ten-second cut fits a pane at its default width.
            seconds_per_pixel: 0.05,
            tool: TimelineTool::Select,
            snap: true,
            quality: 1,
            playing: false,
            transport: slint::Timer::default(),
            clipboard: None,
            preview: slint::Image::default(),
            preview_busy: false,
            preview_wanted: false,
            preview_failed: false,
            export: ExportState::default(),
            settings: SettingsState::default(),
            transcribers: Vec::new(),
            voices: Vec::new(),
            av_token: 0,
            open_menu: -1,
            menu_bar_token: 0,
            menu_target: None,
            menu_token: 0,
            toast: ToastState::default(),
            on_start: true,
            start: StartState::default(),
            recents,
            posters: HashMap::new(),
            posters_pending: HashSet::new(),
            project_name: "Untitled project".into(),
            dock: default_dock(),
            workspace: (0.0, 0.0),
            divider_press: None,
            gesture: Gesture::None,
            stage_guides: Vec::new(),
            inspector_jump: (0, "", ""),
            last_commit: None,
            title_blocks: HashMap::new(),
            drop: None,
            host,
        };
        studio.settings.language = studio.prefs.language.unwrap_or(0);
        studio.settings.transcribe_language = studio.prefs.transcribe_language.unwrap_or(0);
        studio.refresh_models();
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

    /// The window's frame rate, from the session's settings.
    pub fn frame_rate(&self) -> f32 {
        self.session
            .as_ref()
            .map(|session| {
                let settings = session.settings();
                settings.rate_num as f32 / settings.rate_den.max(1) as f32
            })
            .unwrap_or(30.0)
    }

    /// The output size, from the session's settings.
    pub fn output_size(&self) -> (u32, u32) {
        self.session
            .as_ref()
            .map(|session| (session.settings().width, session.settings().height))
            .unwrap_or((1920, 1080))
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
        self.lane_view.get(track_id).is_some_and(|view| view.locked)
    }

    fn lane_size(&self, track_id: &str) -> TrackSize {
        self.lane_view
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
                        model::ClipKind::Audio | model::ClipKind::Text => LANE_MEDIUM,
                        model::ClipKind::Layer => LANE_SMALL,
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
        if !self.snap {
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

    /// The bookkeeping every change to the edit needs: the caches that
    /// follow the document, the autosave, the monitor and the mix.
    fn after_change(&mut self) {
        self.dirty = true;
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
    }

    pub fn undo(&mut self) {
        self.echo = None;
        if let Some(session) = self.session.as_mut()
            && session.can_undo()
        {
            session.undo();
            self.after_change();
        }
    }

    pub fn redo(&mut self) {
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
        let (path, document) = session.prepare_save(None, None, None);
        self.dirty = false;
        spawn(
            move || projects::save(&path, &document),
            move |studio, _, _, result| match result {
                Ok(()) if announce => studio.notify("Project saved", false),
                Ok(()) => {}
                Err(error) => {
                    studio.dirty = true;
                    studio.notify(&format!("Could not save: {error}"), true);
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
                start: piece.start,
                duration: piece.duration,
                source_start: piece.source_start,
                volume: piece.volume as f32,
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

    /// Asks the engine for the frame at the playhead, one at a time: a
    /// request while one is out waits for it, and the newest wins.
    pub fn request_preview(&mut self) {
        /// A monitor frame on its way to the window.
        enum Picture {
            /// Already on the GPU, on the window's own device.
            Texture(concat_host::preview::wgpu::Texture),
            /// Raw RGBA, to be uploaded.
            Pixels(Vec<u8>, u32, u32),
        }

        if self.on_start || self.session.is_none() {
            return;
        }
        if self.preview_busy {
            self.preview_wanted = true;
            return;
        }
        let Some(session) = self.session.as_ref() else {
            return;
        };
        // The echo when there is one: a picture being dragged on the stage
        // is drawn where the pointer has it, not where the document last
        // had it. Same flattening the session does for itself.
        let mut clips = concat_export::flatten::flatten_timeline(self.project(), None);
        let (width, height) = self.output_size();
        // Titles, painted to pictures and rejoined; see concat-host's titles.
        for title in self.host.titles.clips(self.project(), width, height) {
            self.title_blocks.insert(title.clip_id, title.block);
            clips.push(title.clip);
        }
        let scale = match self.quality {
            0 => 1.0,
            1 => 0.5,
            _ => 0.25,
        };
        let width = ((f64::from(width) * scale).round() as u32).max(2) & !1;
        let height = ((f64::from(height) * scale).round() as u32).max(2) & !1;
        let spec = FrameSpec {
            time: f64::from(self.playhead),
            width,
            height,
        };
        let settings = session.settings().clone();
        let monitor = self.host.monitor.clone();
        self.preview_busy = true;
        self.preview_wanted = false;
        spawn(
            move || {
                // On the window's device the frame stays a texture; without
                // one it comes back as pixels and is uploaded here.
                let frame = if monitor.has_gpu() {
                    monitor
                        .frame_texture(clips.clone(), &settings, spec)
                        .map(Picture::Texture)
                } else {
                    monitor
                        .frame(clips.clone(), &settings, spec)
                        .map(|bytes| Picture::Pixels(bytes, width, height))
                };
                // Decode-ahead for whatever comes next, while the pool is warm.
                monitor.prefetch(clips, &settings, spec, 2);
                frame
            },
            |studio, _, _, result| {
                studio.preview_busy = false;
                match result {
                    Ok(Picture::Texture(texture)) => match slint::Image::try_from(texture) {
                        Ok(image) => studio.preview = image,
                        Err(error) => eprintln!("concat: preview texture: {error}"),
                    },
                    Ok(Picture::Pixels(bytes, width, height)) => {
                        let buffer =
                            slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                                &bytes, width, height,
                            );
                        studio.preview = slint::Image::from_rgba8(buffer);
                    }
                    Err(error) => {
                        eprintln!("concat: preview: {error}");
                        if !studio.preview_failed {
                            studio.preview_failed = true;
                            studio.notify(&format!("Preview failed: {error}"), true);
                        }
                    }
                }
                if studio.preview_wanted {
                    studio.request_preview();
                }
            },
        );
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
    pub fn seek(&mut self, seconds: f32) {
        self.playhead = seconds.clamp(0.0, self.duration().max(0.0));
        self.host.playback.seek(f64::from(self.playhead));
        self.request_preview();
    }

    // ── the bin ──

    /// Gives every media item the integer row Slint knows it by. New items
    /// get the next number; nothing is ever renumbered.
    fn assign_media_rows(&mut self) {
        let ids: Vec<String> = self
            .project()
            .media
            .iter()
            .map(|item| item.id.clone())
            .collect();
        for id in ids {
            if !self.media_rows.contains_key(&id) {
                self.media_rows.insert(id, self.next_media_row);
                self.next_media_row += 1;
            }
        }
    }

    pub fn media_by_row(&self, row: i32) -> Option<&model::MediaItem> {
        let id = self.media_rows.iter().find(|(_, held)| **held == row)?.0;
        self.project().media_by_id(id)
    }

    fn shows(filter: MediaFilter, kind: model::MediaKind) -> bool {
        match filter {
            MediaFilter::All => true,
            MediaFilter::Video => kind == model::MediaKind::Video,
            MediaFilter::Audio => kind == model::MediaKind::Audio,
            MediaFilter::Images => kind == model::MediaKind::Image,
        }
    }

    pub fn set_media_filter(&mut self, filter: MediaFilter) {
        self.media_filter = filter;
    }

    pub fn media_select(&mut self, row: i32, additive: bool) {
        let Some(id) = self.media_by_row(row).map(|item| item.id.clone()) else {
            return;
        };
        if additive {
            if !self.media_selected.remove(&id) {
                self.media_selected.insert(id);
            }
        } else {
            self.media_selected.clear();
            self.media_selected.insert(id);
        }
    }

    /// A marquee closed over the grid, as the block of cells it caught. The
    /// walk is over the filtered order, because that is what the grid was
    /// laid out from.
    pub fn media_band(
        &mut self,
        columns: i32,
        from_col: i32,
        to_col: i32,
        from_row: i32,
        to_row: i32,
        additive: bool,
    ) {
        let filter = self.media_filter;
        let mut cell = 0;
        let mut next = if additive {
            self.media_selected.clone()
        } else {
            HashSet::new()
        };
        for item in &self.project().media {
            if !Self::shows(filter, item.kind) {
                continue;
            }
            let (row, col) = (cell / columns.max(1), cell % columns.max(1));
            if row >= from_row && row <= to_row && col >= from_col && col <= to_col {
                next.insert(item.id.clone());
            }
            cell += 1;
        }
        self.media_selected = next;
    }

    pub fn media_remove(&mut self, row: i32) {
        if let Some(id) = self.media_by_row(row).map(|item| item.id.clone()) {
            self.media_selected.remove(&id);
            self.apply(Command::RemoveMedia { media_id: id });
        }
    }

    pub fn media_remove_selected(&mut self) {
        let doomed: Vec<String> = self.media_selected.drain().collect();
        if doomed.is_empty() {
            return;
        }
        self.apply(Command::Batch {
            commands: doomed
                .into_iter()
                .map(|media_id| Command::RemoveMedia { media_id })
                .collect(),
        });
    }

    /// Probes the files on a worker and adds what probed as media.
    pub fn import(&mut self, paths: Vec<std::path::PathBuf>) {
        if paths.is_empty() || self.session.is_none() {
            return;
        }
        spawn(
            move || {
                paths
                    .iter()
                    .map(|path| media::probe(&path.to_string_lossy()))
                    .collect::<Vec<_>>()
            },
            |studio, _, _, results| {
                let mut commands = Vec::new();
                let mut failures = Vec::new();
                for result in results {
                    match result {
                        Ok(summary) => commands.push(Command::AddMedia {
                            item: summary.to_new_media(),
                        }),
                        Err(error) => failures.push(error),
                    }
                }
                let added = commands.len();
                if !commands.is_empty() {
                    studio.apply(Command::Batch { commands });
                }
                if let Some(error) = failures.first() {
                    studio.notify(&crate::host::probe_error(error), true);
                } else if added > 0 {
                    studio.notify(
                        &format!("Imported {added} file{}", if added == 1 { "" } else { "s" }),
                        false,
                    );
                }
            },
        );
    }

    /// Decodes art for every media item that has none yet.
    fn request_media_art(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project_path = session.path().to_owned();
        let wanted: Vec<(String, String, model::MediaKind, bool, Option<f64>)> = self
            .project()
            .media
            .iter()
            .filter(|item| !item.placeholder && !item.path.is_empty())
            .filter(|item| !self.art_pending.contains(&item.id))
            .filter(|item| {
                let needs_thumb = item.kind != model::MediaKind::Audio
                    && (!self.thumbs.contains_key(&item.id) || !self.strips.contains_key(&item.id));
                let needs_peaks = (item.kind == model::MediaKind::Audio || item.has_audio)
                    && !self.peaks.contains_key(&item.id);
                needs_thumb || needs_peaks
            })
            .map(|item| {
                (
                    item.id.clone(),
                    item.path.clone(),
                    item.kind,
                    item.has_audio,
                    item.duration,
                )
            })
            .collect();
        for (id, path, kind, has_audio, duration) in wanted {
            self.art_pending.insert(id.clone());
            let project = project_path.clone();
            spawn(
                move || media_art(id, path, kind, has_audio, duration, project),
                |studio, _, _, art: MediaArt| {
                    studio.art_pending.remove(&art.id);
                    if let Some(frame) = art.thumbnail {
                        studio.thumbs.insert(art.id.clone(), image_of(&frame));
                    }
                    if let Some((frame, frames)) = art.strip {
                        let strip = Strip {
                            image: image_of(&frame),
                            frames: frames as i32,
                            frame_width: (frame.width() / frames.max(1)) as i32,
                            height: frame.height() as i32,
                        };
                        studio.strips.insert(art.id.clone(), strip);
                    }
                    if let Some(peaks) = art.peaks {
                        studio.peaks.insert(art.id.clone(), peaks);
                        let prefix = format!("{}|", art.id);
                        studio
                            .waves
                            .borrow_mut()
                            .retain(|key, _| !key.starts_with(&prefix));
                    }
                },
            );
        }
    }

    /// The clip's filmstrip, with the window of the strip its cut covers:
    /// where in the footage it starts and how much of the footage it spans,
    /// both as fractions, so the lane can pick the frame under each tile
    /// without knowing about speed or source time.
    fn strip_of(&self, clip: &Clip) -> StripData {
        if !matches!(clip.kind, model::ClipKind::Video | model::ClipKind::Image) {
            return StripData::default();
        }
        let Some(strip) = self.strips.get(&clip.media_id) else {
            return StripData::default();
        };
        let footage = self
            .project()
            .media
            .iter()
            .find(|item| item.id == clip.media_id)
            .and_then(|item| item.duration)
            .filter(|seconds| *seconds > 0.0);
        let (start, span) = match footage {
            Some(seconds) => (
                (clip.source_start / seconds).clamp(0.0, 1.0),
                (clip.duration * clip.speed / seconds).clamp(0.0, 1.0),
            ),
            // A still, or footage of unknown length: one frame, all of it.
            None => (0.0, 1.0),
        };
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
    /// one per pointer event.
    fn wave(&self, clip: &Clip) -> SharedString {
        let Some(peaks) = self.peaks.get(&clip.media_id) else {
            return SharedString::new();
        };
        let step = |seconds: f32| (seconds * WAVE_STEPS).round() / WAVE_STEPS;
        let (source_start, duration) = (step(clip.source_start as f32), step(clip.duration as f32));
        let gain = clip.volume as f32;
        let key = format!(
            "{}|{:.3}|{:.3}|{:.3}",
            clip.media_id, source_start, duration, gain
        );
        if let Some(cached) = self.waves.borrow().get(&key) {
            return cached.clone();
        }
        let built = SharedString::from(wave_path(peaks, source_start, duration, gain));
        self.waves.borrow_mut().insert(key, built.clone());
        built
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
                let item = self.media_by_row(id.parse().ok()?)?;
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
            "text" => Some(DropPlan {
                kind: ClipKind::Text,
                label: label.to_owned(),
                media: String::new(),
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
            .snapped(seconds.max(0.0), 8.0 * self.seconds_per_pixel, "")
            .max(0.0);
        Some(plan)
    }

    /// Commits a plan that has a lane, and selects what it made.
    pub fn place(&mut self, plan: &DropPlan) {
        let Some(track_id) = self.row_track(plan.row).map(|track| track.id.clone()) else {
            return;
        };
        let created = if plan.kind == ClipKind::Text {
            self.apply(Command::AddTextClip {
                track_id: Some(track_id),
                start: f64::from(plan.start),
                style: Some(new_title_style()),
                duration: Some(f64::from(plan.duration)),
                offset_y: None,
            })
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
            self.apply(Command::AddTextClip {
                track_id: None,
                start,
                style: Some(new_title_style()),
                duration: Some(f64::from(plan.duration)),
                offset_y: None,
            })
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

    /// The selected clip's id, when exactly one is selected.
    pub fn sole_selection(&self) -> Option<String> {
        (self.selection.len() == 1).then(|| self.selection[0].clone())
    }

    /// A catalogue filter or effect, applied to the selected clip's chain.
    pub fn apply_catalogue(&mut self, id: &str, video: bool) {
        let Some(clip_id) = self.sole_selection() else {
            self.notify("Select a clip on the timeline first", true);
            return;
        };
        let Some(clip) = self.clip(&clip_id).cloned() else {
            return;
        };
        if video && !clip.kind.is_visual() {
            self.notify("Select a video or image clip on the timeline first", true);
            return;
        }
        if !video && clip.kind == model::ClipKind::Image {
            self.notify("A still has no sound to filter", true);
            return;
        }
        let entry = AppliedFilter {
            id: id.to_owned(),
            params: std::collections::BTreeMap::new(),
            enabled: true,
        };
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
        let is_look = Catalogue::builtin()
            .packages()
            .find(|package| package.answers_to(id))
            .is_some_and(|package| package.kind() == PackageKind::Filter);
        self.inspector_jump = (
            self.inspector_jump.0 + 1,
            if video { "Effects" } else { "Audio" },
            if !video {
                "Sound"
            } else if is_look {
                "Filters"
            } else {
                "Effects"
            },
        );
    }

    pub fn apply_transition(&mut self, id: &str) {
        let Some(clip_id) = self.sole_selection() else {
            self.notify("Select the clip the transition leads into", true);
            return;
        };
        if !self
            .clip(&clip_id)
            .is_some_and(|clip| clip.kind.is_visual())
        {
            self.notify("Select a video or image clip on the timeline first", true);
            return;
        }
        self.apply(Command::UpdateClip {
            clip_id,
            patch: ClipPatch {
                transition_in: Some(Some(Transition {
                    id: id.to_owned(),
                    duration: 0.5,
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
        self.begin_echo();
        let Some(clip) = self.echo_clip_mut(&id) else {
            return;
        };
        if !clip.kind.is_visual() {
            return;
        }
        let at = match clip
            .video_effects
            .iter()
            .position(|entry| entry.id == ADJUST_ID)
        {
            Some(at) => at,
            None => {
                clip.video_effects.insert(
                    0,
                    AppliedFilter {
                        id: ADJUST_ID.to_owned(),
                        params: std::collections::BTreeMap::new(),
                        enabled: true,
                    },
                );
                0
            }
        };
        clip.video_effects[at]
            .params
            .insert(key.to_owned(), f64::from(value));
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
        let moving = if self.selection.iter().any(|held| held == id) {
            self.selection.clone()
        } else {
            vec![id.to_owned()]
        };
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
                let threshold = 8.0 * self.seconds_per_pixel;
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
                let threshold = 8.0 * self.seconds_per_pixel;
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
            // A stage gesture is the monitor's; the lanes have nothing to
            // add to it.
            Gesture::None
            | Gesture::StageMove { .. }
            | Gesture::StageScale { .. }
            | Gesture::StageRotate { .. } => {}
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
                    });
                }
            }
            Gesture::None => {
                self.echo = None;
            }
            // Not a lane gesture: hand it back untouched, echo and all.
            other @ (Gesture::StageMove { .. }
            | Gesture::StageScale { .. }
            | Gesture::StageRotate { .. }) => {
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
        self.begin_echo();
        let value = f64::from(value);
        let Some(clip) = self.echo_clip_mut(&id) else {
            return;
        };
        let text = clip.text.get_or_insert_with(TextStyle::default);
        match field {
            ClipField::Scale => clip.scale = value.clamp(0.05, 8.0),
            ClipField::OffsetX => clip.offset_x = value.clamp(-1.0, 1.0),
            ClipField::OffsetY => clip.offset_y = value.clamp(-1.0, 1.0),
            ClipField::Rotation => clip.rotation = value.clamp(-180.0, 180.0),
            ClipField::Opacity => clip.opacity = value.clamp(0.0, 1.0),
            ClipField::Volume => clip.volume = value.max(0.0),
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
            ClipField::AnimIn => set_slot(
                concat_project::model::AnimationSlot::In,
                &mut clip.animation_in,
                value,
                0.5,
            ),
            ClipField::AnimOut => set_slot(
                concat_project::model::AnimationSlot::Out,
                &mut clip.animation_out,
                value,
                0.5,
            ),
            ClipField::AnimCombo => set_slot(
                concat_project::model::AnimationSlot::Combo,
                &mut clip.animation_combo,
                value,
                clip.duration,
            ),
            ClipField::AnimInDuration => {
                if let Some(set) = clip.animation_in.as_mut() {
                    set.duration = value.clamp(0.05, 60.0);
                }
            }
            ClipField::AnimOutDuration => {
                if let Some(set) = clip.animation_out.as_mut() {
                    set.duration = value.clamp(0.05, 60.0);
                }
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
    }

    pub fn clip_set_colour(&mut self, field: ClipTextField, value: slint::Color) {
        let Some(id) = self.sole_selection() else {
            return;
        };
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
            ClipTextField::StrokeColor => text.stroke_color = hex_of(value),
            ClipTextField::Background => text.background = hex_with_alpha(value),
            _ => {}
        }
    }

    /// The inspector's gesture is over: what differs between the echo and
    /// the session becomes commands, as one batch.
    pub fn clip_commit(&mut self) {
        let Some(id) = self.sole_selection() else {
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
        {
            commands.push(Command::SetClipTransform {
                clip_id: id.clone(),
                scale: Some(after.scale),
                offset_x: Some(after.offset_x),
                offset_y: Some(after.offset_y),
                rotation: Some(after.rotation),
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
        for (slot, now, was) in [
            (
                concat_project::model::AnimationSlot::In,
                &after.animation_in,
                &before.animation_in,
            ),
            (
                concat_project::model::AnimationSlot::Out,
                &after.animation_out,
                &before.animation_out,
            ),
            (
                concat_project::model::AnimationSlot::Combo,
                &after.animation_combo,
                &before.animation_combo,
            ),
        ] {
            if now != was {
                commands.push(Command::SetClipAnimation {
                    clip_id: id.clone(),
                    slot,
                    animation: now.clone(),
                });
            }
        }
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
        // moment of it, takes the last one's place. Every command here sets
        // absolute values, so undoing the previous and applying this lands
        // where this alone would have.
        let key = format!("{}:{}", after.id, commit_key(&commands));
        let now = std::time::Instant::now();
        let coalesce = self
            .last_commit
            .as_ref()
            .is_some_and(|(last, at)| *last == key && now.duration_since(*at).as_millis() < 900);
        if coalesce
            && let Some(session) = self.session.as_mut()
            && session.can_undo()
        {
            session.undo();
        }
        match commands.len() {
            1 => {
                self.apply(commands.remove(0));
            }
            _ => {
                self.apply(Command::Batch { commands });
            }
        }
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
        let (w, h) = if let Some((bw, bh)) = painted {
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
        // The placement at the playhead: the clip's own, moved by its
        // animation, so the box follows a slide or a spin.
        let base = concat_core::timeline::Transform {
            scale: clip.scale,
            offset_x: clip.offset_x,
            offset_y: clip.offset_y,
            rotation: clip.rotation,
        };
        let placed = match concat_project::animation::animation_of(clip) {
            Some(animation) if clip.duration > 0.0 => {
                let x = ((f64::from(self.playhead) - clip.start) / clip.duration).clamp(0.0, 1.0);
                animation.transform_at(base, x)
            }
            _ => base,
        };
        let factor = placed.scale / clip.scale.max(1e-6);
        Footprint {
            cx: 0.5 + placed.offset_x,
            cy: 0.5 + placed.offset_y,
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
        let (x, y) = (f64::from(x), f64::from(y));
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
            .map(|clip| StageOrigin {
                clip: clip.id.clone(),
                offset_x: clip.offset_x,
                offset_y: clip.offset_y,
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
        self.begin_echo();
        self.gesture = if grip == 4 {
            Gesture::StageRotate {
                clip: id.to_owned(),
                rotation: clip.rotation,
                centre,
                from: dy.atan2(dx),
            }
        } else {
            Gesture::StageScale {
                clip: id.to_owned(),
                scale: clip.scale,
                centre,
                from: dx.hypot(dy).max(1.0),
            }
        };
    }

    /// The pointer moved with a stage gesture live: the echo follows, and
    /// the monitor composites it.
    pub fn stage_dragged(&mut self, x: f32, y: f32, snap: bool) {
        let (x, y) = (f64::from(x), f64::from(y));
        let (width, height) = self.output_size();
        let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
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
                if self.snap {
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
                for origin in origins {
                    if let Some(clip) = self.echo_clip_mut(&origin.clip) {
                        clip.offset_x = (origin.offset_x + dx).clamp(-1.0, 1.0);
                        clip.offset_y = (origin.offset_y + dy).clamp(-1.0, 1.0);
                    }
                }
            }
            Gesture::StageScale {
                clip,
                scale,
                centre,
                from,
            } => {
                let dx = x * f64::from(width) - centre.0;
                let dy = y * f64::from(height) - centre.1;
                let mut next = scale * dx.hypot(dy) / from;
                if snap {
                    next = (next * 20.0).round() / 20.0;
                }
                if let Some(clip) = self.echo_clip_mut(clip) {
                    clip.scale = next.clamp(0.05, 8.0);
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
                if let Some(clip) = self.echo_clip_mut(clip) {
                    clip.rotation = next;
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
        let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        let touched: Vec<String> = match gesture {
            Gesture::StageMove { origins, .. } => {
                origins.into_iter().map(|origin| origin.clip).collect()
            }
            Gesture::StageScale { clip, .. } | Gesture::StageRotate { clip, .. } => vec![clip],
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
            {
                commands.push(Command::SetClipTransform {
                    clip_id: id,
                    scale: Some(after.scale),
                    offset_x: Some(after.offset_x),
                    offset_y: Some(after.offset_y),
                    rotation: Some(after.rotation),
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

    pub fn delete_selected(&mut self) {
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
            self.apply(Command::RemoveClips { clip_ids: doomed });
        }
        self.selection.clear();
    }

    pub fn merge_blocked(&self) -> Option<String> {
        if self.selection.len() != 2 {
            return Some("Select two clips to merge".to_owned());
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

    /// A copy of `source` laid after it. Three commands, because a clip's
    /// in-point and length are set by trims, not by placement.
    pub fn duplicate(&mut self, source: &Clip) {
        let end = source.start + source.duration;
        if source.kind == model::ClipKind::Text {
            let created = self.apply(Command::AddTextClip {
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
            });
        }
        let after_head = placed.duration - head.max(0.0);
        let tail = source.duration - after_head;
        if tail.abs() > 1e-6 {
            commands.push(Command::TrimClip {
                clip_id: created.clone(),
                edge: TrimEdge::End,
                delta: tail,
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

    /// Sound to detach, or a title's words to speak - the tray only hangs
    /// the A/V menu when the selection has something to offer it.
    pub fn has_av_tools(&self) -> bool {
        self.sole_selection()
            .and_then(|id| {
                self.clip(&id)
                    .map(|clip| clip.kind != model::ClipKind::Image)
            })
            .unwrap_or(false)
    }

    // ── projects ──

    /// Opens a project as the session and leaves the launch screen.
    pub fn open_project(&mut self, info: ProjectInfo) {
        match Session::open_info(&info) {
            Ok(session) => {
                if let Err(error) = projects::remember(&self.host.dirs.config, &info) {
                    eprintln!("concat: {error}");
                }
                self.pause();
                self.session = Some(session);
                self.echo = None;
                self.dirty = false;
                self.project_name = info.name.clone();
                self.export.name = projects::folder_name(&info.name);
                self.selection.clear();
                self.media_selected.clear();
                self.lane_view.clear();
                self.playhead = 0.0;
                self.scroll_left = 0.0;
                self.on_start = false;
                self.preview_failed = false;
                self.start.busy = false;
                self.start.error.clear();
                self.recents = projects::list(&self.host.dirs.config);
                self.host.monitor.clear();
                self.sync_audio();
                self.request_media_art();
                self.request_preview();
            }
            Err(error) => {
                self.start.busy = false;
                self.start.error = error;
            }
        }
    }

    pub fn create_project(&mut self) {
        let name = self.start.name.trim().to_owned();
        let name = if name.is_empty() {
            "Untitled project".to_owned()
        } else {
            name
        };
        let (_, width, height) = RESOLUTIONS[self.start.resolution.min(RESOLUTIONS.len() - 1)];
        let (_, num, den) = START_RATES[self.start.rate.min(START_RATES.len() - 1)];
        if self.start.location.trim().is_empty() {
            self.start.error = "Choose where the project folder should go".into();
            return;
        }
        match projects::create(&self.start.location, &name, width, height, num, den) {
            Ok(info) => self.open_project(info),
            Err(error) => self.start.error = error,
        }
    }

    pub fn open_recent(&mut self, path: &str) {
        match projects::open(path) {
            Ok(info) => self.open_project(info),
            Err(error) => self.start.error = error,
        }
    }

    pub fn forget_recent(&mut self, path: &str) {
        if let Err(error) = projects::forget(&self.host.dirs.config, path) {
            self.start.error = error;
        }
        self.recents = projects::list(&self.host.dirs.config);
    }

    /// Saves, then closes the session and returns to the launch screen.
    pub fn close_project(&mut self) {
        self.pause();
        if let Some(session) = self.session.as_mut() {
            let (path, document) = session.prepare_save(None, None, None);
            if let Err(error) = projects::save(&path, &document) {
                self.notify(&format!("Could not save: {error}"), true);
                return;
            }
        }
        self.autosave.stop();
        self.session = None;
        self.echo = None;
        self.dirty = false;
        self.selection.clear();
        self.gesture = Gesture::None;
        self.preview = slint::Image::default();
        self.host.monitor.clear();
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
                    let cached = std::path::Path::new(&path)
                        .join("cache")
                        .join("preview.jpg");
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

    // ── export ──

    /// The frame the export renders at: the sheet's short side, scaled
    /// along the project's aspect and rounded to even dimensions, which is
    /// what the encoder's chroma subsampling needs.
    pub fn export_size(&self) -> (u32, u32) {
        let short = EXPORT_SHORT_SIDES[self.export.resolution.min(EXPORT_SHORT_SIDES.len() - 1)];
        let (project_w, project_h) = self.output_size();
        let (project_w, project_h) = (project_w.max(1) as f64, project_h.max(1) as f64);
        let even = |side: f64| ((side / 2.0).round() as u32 * 2).max(2);
        if project_w >= project_h {
            (even(short as f64 * project_w / project_h), short)
        } else {
            (short, even(short as f64 * project_h / project_w))
        }
    }

    pub fn export_size_bytes(&self, tier: usize) -> f32 {
        let (width, height) = self.export_size();
        let (num, den) = EXPORT_RATES[self.export.rate.min(2)];
        let rate = num as f32 / den as f32;
        let pixels = (width as f32 * height as f32) / (1920.0 * 1080.0);
        let video = EXPORT_TIERS[tier.min(2)] * 1_000_000.0 * pixels * (rate / 30.0);
        (video + AUDIO_BPS) * self.duration().max(1.0) / 8.0
    }

    /// Starts the render on a worker, reporting into the sheet.
    pub fn export_start(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        if self.timeline().clips.is_empty() {
            self.export.phase = ExportPhase::Failed;
            self.export.message = "There is nothing on the timeline to export".into();
            return;
        }
        let job = match self.host.exporter.begin() {
            Ok(job) => job,
            Err(error) => {
                self.export.phase = ExportPhase::Failed;
                self.export.message = error;
                return;
            }
        };
        let output = format!(
            "{}/{}.mp4",
            self.export.folder.trim_end_matches('/'),
            self.export.name.trim()
        );
        let spec = ExportSpec {
            output: output.clone(),
            crf: EXPORT_CRF[self.export.quality.min(2)],
            preset: "medium".into(),
        };
        let (frame_w, frame_h) = self.output_size();
        let titles = self
            .host
            .titles
            .clips(session.project(), frame_w, frame_h)
            .into_iter()
            .map(|title| title.clip)
            .collect();
        let mut request = export::request(session, &spec, titles);
        let (width, height) = self.export_size();
        let (num, den) = EXPORT_RATES[self.export.rate.min(2)];
        request.width = width;
        request.height = height;
        request.rate_num = num;
        request.rate_den = den;

        self.pause();
        self.export.phase = ExportPhase::Running;
        self.export.progress = 0.0;
        self.export.stage = "Rendering video".into();
        self.export.message.clear();
        self.export.written.clear();

        spawn(
            move || {
                let job = job;
                export::run(&request, job.cancel_flag(), |progress| {
                    let fraction = if progress.total > 0 {
                        progress.frame as f32 / progress.total as f32
                    } else {
                        0.0
                    };
                    let stage = match progress.stage {
                        "rendering" => "Rendering video",
                        "mixing audio" => "Mixing audio",
                        "muxing" => "Finalising file",
                        other => other,
                    }
                    .to_owned();
                    on_ui(move |studio, _, _| {
                        if studio.export.phase == ExportPhase::Running {
                            studio.export.progress = fraction.clamp(0.0, 1.0);
                            studio.export.stage = stage;
                        }
                    });
                })
            },
            |studio, _, _, result| match result {
                Ok(written) => {
                    studio.export.phase = ExportPhase::Done;
                    studio.export.progress = 1.0;
                    studio.export.written = written;
                    studio.notify("Export finished", false);
                }
                Err(error) => {
                    if studio.export.phase == ExportPhase::Idle {
                        // Cancelled: the sheet already went back to idle.
                        return;
                    }
                    studio.export.phase = ExportPhase::Failed;
                    studio.export.message = error.clone();
                    studio.notify(&format!("Export failed: {error}"), true);
                }
            },
        );
    }

    pub fn export_cancel(&mut self) {
        self.host.exporter.cancel();
        self.export.phase = ExportPhase::Idle;
        self.export.progress = 0.0;
    }

    // ── speech ──

    /// The settings sheet's two model lists, from what is on disk.
    pub fn refresh_models(&mut self) {
        let dirs = &self.host.dirs;
        let downloading: HashMap<String, (Option<f32>, bool)> = self
            .transcribers
            .iter()
            .chain(self.voices.iter())
            .filter(|model| model.fetched.is_some())
            .map(|model| (model.id.clone(), (model.fetched, model.unpacking)))
            .collect();
        let chosen_transcriber = self.prefs.transcriber_model.clone();
        let chosen_voice = self.prefs.tts_model.clone();
        if let Ok(status) = concat_speech::Transcriber::status(dirs) {
            self.transcribers = status
                .models
                .iter()
                .map(|model| {
                    let (fetched, unpacking) =
                        downloading.get(&model.id).copied().unwrap_or((None, false));
                    ModelState {
                        id: model.id.clone(),
                        name: model.label.clone(),
                        note: model.blurb.clone(),
                        megabytes: model.size_bytes as f32 / 1_000_000.0,
                        accuracy: if model.id.starts_with("tiny") {
                            2
                        } else if model.id.starts_with("base") {
                            3
                        } else {
                            4
                        },
                        installed: model.downloaded,
                        active: chosen_transcriber.as_deref() == Some(model.id.as_str()),
                        fetched,
                        unpacking,
                    }
                })
                .collect();
        }
        if let Ok(status) = concat_speech::Speech::status(dirs) {
            self.voices = status
                .models
                .iter()
                .map(|model| {
                    let (fetched, unpacking) =
                        downloading.get(&model.id).copied().unwrap_or((None, false));
                    ModelState {
                        id: model.id.clone(),
                        name: model.label.clone(),
                        note: model.blurb.clone(),
                        megabytes: model.size_bytes as f32 / 1_000_000.0,
                        accuracy: if model.id.contains("int8") { 4 } else { 5 },
                        installed: model.downloaded,
                        active: chosen_voice.as_deref() == Some(model.id.as_str()),
                        fetched,
                        unpacking,
                    }
                })
                .collect();
        }
        // An engine with nothing chosen falls back to whatever is installed,
        // rather than silently having no model at all.
        for list in [&mut self.transcribers, &mut self.voices] {
            if !list.iter().any(|model| model.active && model.installed)
                && let Some(first) = list.iter_mut().find(|model| model.installed)
            {
                first.active = true;
            }
        }
    }

    fn is_transcriber(&self, id: &str) -> bool {
        self.transcribers.iter().any(|model| model.id == id)
    }

    pub fn model_activate(&mut self, id: &str) {
        if self.is_transcriber(id) {
            self.prefs.transcriber_model = Some(id.to_owned());
        } else {
            self.prefs.tts_model = Some(id.to_owned());
        }
        self.prefs.save(&self.host.dirs);
        self.refresh_models();
    }

    pub fn model_download(&mut self, id: &str) {
        let transcriber = self.is_transcriber(id);
        let list = if transcriber {
            &mut self.transcribers
        } else {
            &mut self.voices
        };
        let Some(model) = list.iter_mut().find(|model| model.id == id) else {
            return;
        };
        if model.installed || model.fetched.is_some() {
            return;
        }
        model.fetched = Some(0.0);
        let id = id.to_owned();
        let dirs = self.host.dirs.clone();
        let whisper = Arc::clone(&self.host.transcriber);
        let kokoro = Arc::clone(&self.host.speech);
        spawn(
            move || {
                let report = |progress: concat_speech::DownloadProgress| {
                    on_ui(move |studio, _, _| {
                        for list in [&mut studio.transcribers, &mut studio.voices] {
                            if let Some(model) =
                                list.iter_mut().find(|model| model.id == progress.id)
                            {
                                model.fetched = Some(progress.received as f32 / 1_000_000.0);
                                model.unpacking = progress.unpacking;
                                if progress.total > 0 {
                                    model.megabytes = progress.total as f32 / 1_000_000.0;
                                }
                            }
                        }
                    });
                };
                let result = if transcriber {
                    whisper.download_model(&dirs, &id, report)
                } else {
                    kokoro.download_model(&dirs, &id, report)
                };
                (id, result)
            },
            |studio, _, _, (id, result)| {
                for list in [&mut studio.transcribers, &mut studio.voices] {
                    if let Some(model) = list.iter_mut().find(|model| model.id == id) {
                        model.fetched = None;
                        model.unpacking = false;
                    }
                }
                match result {
                    Ok(()) => {
                        studio.notify("Model ready", false);
                        if studio.is_transcriber(&id) && studio.prefs.transcriber_model.is_none() {
                            studio.prefs.transcriber_model = Some(id.clone());
                        } else if !studio.is_transcriber(&id) && studio.prefs.tts_model.is_none() {
                            studio.prefs.tts_model = Some(id.clone());
                        }
                        studio.prefs.save(&studio.host.dirs);
                    }
                    Err(error) => studio.notify(&error, true),
                }
                studio.refresh_models();
            },
        );
    }

    pub fn model_cancel(&mut self, id: &str) {
        if self.is_transcriber(id) {
            self.host.transcriber.cancel_download();
        } else {
            self.host.speech.cancel_download();
        }
    }

    pub fn model_remove(&mut self, id: &str) {
        let result = if self.is_transcriber(id) {
            self.host.transcriber.delete_model(&self.host.dirs, id)
        } else {
            self.host.speech.delete_model(&self.host.dirs, id)
        };
        if let Err(error) = result {
            self.notify(&error, true);
        }
        self.refresh_models();
    }

    fn language_code(&self) -> &'static str {
        TRANSCRIBE_LANGUAGES
            .get(self.settings.transcribe_language.max(0) as usize)
            .copied()
            .unwrap_or("auto")
    }

    /// Auto captions for the selected clip: the transcription runs on a
    /// worker and lands as one batch of title clips.
    pub fn caption_selected(&mut self) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        let Some(clip) = self.clip(&id).cloned() else {
            return;
        };
        let Some(media) = self.project().media_by_id(&clip.media_id).cloned() else {
            self.notify("This clip has no file to transcribe", true);
            return;
        };
        let Some(model) = self
            .transcribers
            .iter()
            .find(|model| model.active && model.installed)
        else {
            self.notify(
                "Download a transcriber model in Settings > Transcriber first",
                true,
            );
            return;
        };
        let request = concat_speech::transcribe::TranscribeRequest {
            path: media.path.clone(),
            source_start: clip.source_start,
            window: clip.duration * clip.speed,
            language: self.language_code().to_owned(),
            model_id: model.id.clone(),
        };
        let dirs = self.host.dirs.clone();
        let transcriber = Arc::clone(&self.host.transcriber);
        self.notify("Transcribing…", false);
        spawn(
            move || transcriber.transcribe(&dirs, &request, |_| {}),
            move |studio, _, _, result| match result {
                Ok(segments) => {
                    let commands: Vec<Command> = segments
                        .iter()
                        .map(|segment| Command::AddTextClip {
                            track_id: None,
                            start: clip.start + segment.start / clip.speed,
                            style: Some(TextStyle {
                                content: segment.text.clone(),
                                font_family: "Inter".to_owned(),
                                font_size: 0.05,
                                font_weight: 600.0,
                                ..TextStyle::default()
                            }),
                            duration: Some(((segment.end - segment.start) / clip.speed).max(0.2)),
                            offset_y: Some(0.35),
                        })
                        .collect();
                    let count = commands.len();
                    if count == 0 {
                        studio.notify("Nothing was said in that clip", true);
                    } else {
                        studio.apply(Command::Batch { commands });
                        studio.notify(&format!("Added {count} captions"), false);
                    }
                }
                Err(error) => studio.notify(&error, true),
            },
        );
    }

    /// A title's words, spoken: the WAV lands in the project and on the
    /// timeline at the title's start.
    pub fn speak_selected(&mut self) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        let Some(clip) = self.clip(&id).cloned() else {
            return;
        };
        let Some(text) = clip.text.as_ref().map(|text| text.content.clone()) else {
            self.notify("Select a title to speak", true);
            return;
        };
        let Some(model) = self
            .voices
            .iter()
            .find(|model| model.active && model.installed)
        else {
            self.notify("Download a voice model in Settings > Speech first", true);
            return;
        };
        let Some(project) = self
            .session
            .as_ref()
            .map(|session| session.path().to_owned())
        else {
            return;
        };
        let request = concat_speech::tts::SpeakRequest {
            model_id: model.id.clone(),
            voice: self.prefs.tts_voice.unwrap_or(DEFAULT_VOICE),
            text,
            speed: 1.0,
            project,
        };
        let dirs = self.host.dirs.clone();
        let speech = Arc::clone(&self.host.speech);
        self.notify("Generating voice…", false);
        spawn(
            move || {
                let spoken = speech.speak(&dirs, &request, |_| {})?;
                let summary = media::probe(&spoken.path)?;
                Ok::<_, String>(summary)
            },
            move |studio, _, _, result| match result {
                Ok(summary) => {
                    let created = studio.apply(Command::AddMedia {
                        item: summary.to_new_media(),
                    });
                    let media_id = created.or_else(|| {
                        studio
                            .project()
                            .media
                            .iter()
                            .find(|item| item.path == summary.path)
                            .map(|item| item.id.clone())
                    });
                    if let Some(media_id) = media_id {
                        studio.apply(Command::AddClipAtFirstFree {
                            media_id,
                            start: clip.start,
                        });
                        studio.notify("Voice added at the title", false);
                    }
                }
                Err(error) => studio.notify(&error, true),
            },
        );
    }

    /// Packs the open project into the template library.
    pub fn save_template(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let document = session.document();
        let settings = session.settings().clone();
        let path = session.path().to_owned();
        let name = format!("{} template", self.project_name);
        let config = self.host.dirs.config.clone();
        spawn(
            move || templates::save(&config, &document, &settings, &path, &name),
            |studio, _, _, result| match result {
                Ok(info) => studio.notify(&format!("Saved template {:?}", info.name), false),
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
                        name: lane.name.as_str().into(),
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

        sync(
            &models.clips,
            timeline
                .clips
                .iter()
                .map(|clip| ClipData {
                    id: clip.id.as_str().into(),
                    name: clip.name.as_str().into(),
                    kind: kind_of(clip),
                    row: self.row_of(&clip.track_id),
                    start: clip.start as f32,
                    duration: clip.duration as f32,
                    selected: self.selection.iter().any(|id| id == &clip.id),
                    fx: clip.video_effects.iter().any(|effect| effect.enabled),
                    transition_in: clip.transition_in.is_some(),
                    fade_in: clip.fade_in as f32,
                    fade_out: clip.fade_out as f32,
                    volume: clip.volume as f32,
                    text_body: clip
                        .text
                        .as_ref()
                        .map(|text| text.content.as_str())
                        .unwrap_or_default()
                        .into(),
                    wave: if clip.kind == model::ClipKind::Audio {
                        self.wave(clip)
                    } else {
                        SharedString::new()
                    },
                    strip: self.strip_of(clip),
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
        editor.set_playing(self.playing);
        editor.set_preview_frame(self.preview.clone());
        sync(&models.stage, self.stage_items());
        sync(&models.guides, self.stage_guides.clone());

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
        editor.set_inspector_jump_token(self.inspector_jump.0);
        editor.set_inspector_jump_tab(self.inspector_jump.1.into());
        editor.set_inspector_jump_section(self.inspector_jump.2.into());
        let active = project
            .timelines
            .iter()
            .position(|timeline| timeline.id == project.active_timeline_id)
            .unwrap_or(0);
        editor.set_timeline_current_tab(active as i32);
        editor.set_playhead(self.playhead);
        editor.set_scroll_left(self.scroll_left);
        editor.set_seconds_per_pixel(self.seconds_per_pixel);
        editor.set_frame_rate(self.frame_rate());
        editor.set_tool(self.tool);
        editor.set_snap(self.snap);
        editor.set_selected_count(self.selection.len() as i32);
        editor.set_has_av_tools(self.has_av_tools());
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
                Some(clip) if clip.kind.is_visual() => adjust_rows(&clip.video_effects),
                _ => Vec::new(),
            },
        );
    }

    /// The selection, flattened for the inspector: exactly one clip or
    /// nothing.
    fn selected(&self) -> SelectedClipData {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return SelectedClipData::default();
        };
        let text = clip.text.clone().unwrap_or_default();
        let fill = colour_of(&text.color);
        let stroke = colour_of(&text.stroke_color);
        let plate = colour_of(&text.background);
        SelectedClipData {
            present: true,
            id: clip.id.as_str().into(),
            name: clip.name.as_str().into(),
            kind: kind_of(clip),
            duration: clip.duration as f32,
            scale: clip.scale as f32,
            offset_x: clip.offset_x as f32,
            offset_y: clip.offset_y as f32,
            rotation: clip.rotation as f32,
            opacity: clip.opacity as f32,
            volume: clip.volume as f32,
            speed: clip.speed as f32,
            preserve_pitch: clip.preserve_pitch,
            speed_curve: match &clip.speed_curve {
                None => -1,
                Some(points) => concat_project::speed::preset_of(points)
                    .map(|index| index as i32)
                    .unwrap_or(concat_project::speed::PRESETS.len() as i32),
            },
            reverse: clip.reverse,
            anim_in: slot_index(concat_project::model::AnimationSlot::In, &clip.animation_in),
            anim_out: slot_index(
                concat_project::model::AnimationSlot::Out,
                &clip.animation_out,
            ),
            anim_combo: slot_index(
                concat_project::model::AnimationSlot::Combo,
                &clip.animation_combo,
            ),
            anim_in_duration: clip
                .animation_in
                .as_ref()
                .map(|set| set.duration as f32)
                .unwrap_or(0.5),
            anim_out_duration: clip
                .animation_out
                .as_ref()
                .map(|set| set.duration as f32)
                .unwrap_or(0.5),
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
            shadow: text.shadow,
            plate,
            plate_hex: hex_of(plate).into(),
            plated: plate.alpha() > 0,
            line_height: text.line_height as f32,
            tracking: text.tracking as f32,
        }
    }

    /// The menus, the dialogs, the bin and the engine lists.
    pub fn publish_chrome(&self, app: &App, models: &Models) {
        let editor = app.global::<Editor>();

        // The catalogue's shelves. Built in, so they never change after
        // start; `sync` makes republishing them a no-op.
        let (groups, entries) = shelves(PackageKind::Effect);
        sync(&models.effect_groups, groups);
        sync(&models.catalogue_effects, entries);
        let (groups, entries) = shelves(PackageKind::Filter);
        sync(&models.filter_groups, groups);
        sync(&models.catalogue_filters, entries);
        let (groups, entries) = shelves(PackageKind::Audio);
        sync(&models.audio_groups, groups);
        sync(&models.catalogue_audio, entries);

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
        let (_, width, height) = RESOLUTIONS[self.start.resolution.min(RESOLUTIONS.len() - 1)];
        let (_, num, den) = START_RATES[self.start.rate.min(START_RATES.len() - 1)];
        app.set_start(StartData {
            name: self.start.name.as_str().into(),
            location: self.start.location.as_str().into(),
            resolution: self.start.resolution as i32,
            rate: self.start.rate as i32,
            size_readout: format!("{width} x {height}").into(),
            rate_readout: format!("{num}/{den} fps").into(),
            busy: self.start.busy,
            error: self.start.error.as_str().into(),
        });
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

        // The bin.
        let filter = self.media_filter;
        let items = &self.project().media;
        sync(
            &models.media,
            items
                .iter()
                .filter(|item| Self::shows(filter, item.kind))
                .map(|item| MediaItemData {
                    id: *self.media_rows.get(&item.id).unwrap_or(&0),
                    name: item.name.as_str().into(),
                    kind: media_kind_of(item.kind),
                    duration: item.duration.unwrap_or(0.0) as f32,
                    thumbnail: self.thumbs.get(&item.id).cloned().unwrap_or_default(),
                    wave: match self.peaks.get(&item.id) {
                        Some(peaks) if item.kind == model::MediaKind::Audio => {
                            wave_path(peaks, 0.0, item.duration.unwrap_or(0.0) as f32, 1.0).into()
                        }
                        _ => SharedString::new(),
                    },
                    selected: self.media_selected.contains(&item.id),
                })
                .collect(),
        );
        editor.set_media_count_all(items.len() as i32);
        editor.set_media_count_video(
            items
                .iter()
                .filter(|item| item.kind == model::MediaKind::Video)
                .count() as i32,
        );
        editor.set_media_count_audio(
            items
                .iter()
                .filter(|item| item.kind == model::MediaKind::Audio)
                .count() as i32,
        );
        editor.set_media_count_images(
            items
                .iter()
                .filter(|item| item.kind == model::MediaKind::Image)
                .count() as i32,
        );
        editor.set_media_selected_count(self.media_selected.len() as i32);
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
        editor.set_quality_index(self.quality as i32);

        let rows = self.menu();
        editor.set_menu_height(Self::menu_height(&rows));
        sync(&models.menu, rows);
        editor.set_menu_token(self.menu_token);

        app.set_export(self.export_data());
        app.set_settings(SettingsData {
            open: self.settings.open,
            tab: self.settings.tab,
            language: self.settings.language as i32,
            transcribe_language: self.settings.transcribe_language,
            disk: {
                let installed: Vec<&ModelState> = self
                    .transcribers
                    .iter()
                    .chain(self.voices.iter())
                    .filter(|model| model.installed)
                    .collect();
                let on_disk: f32 = installed.iter().map(|model| model.megabytes).sum();
                format!("{} installed · {on_disk:.0} MB on disk", installed.len()).into()
            },
            version: env!("CARGO_PKG_VERSION").into(),
            engine: format!("concat-engine · FFmpeg {}", concat_media::linked_version()).into(),
        });
        sync(&models.transcribers, Self::model_rows(&self.transcribers));
        sync(&models.voices, Self::model_rows(&self.voices));

        let av = self.av_menu();
        editor.set_av_height(Self::menu_height(&av));
        sync(&models.av, av);
        editor.set_av_token(self.av_token);

        let bar = self.menu_bar();
        app.set_app_menu_height(Self::menu_height(&bar));
        sync(&models.bar, bar);
        app.set_app_menu_token(self.menu_bar_token);
        app.set_open_menu(self.open_menu);
    }

    /// Asks the workers for anything the launch screen or the bin is
    /// missing. Separate from `publish` because it mutates.
    pub fn refresh_art(&mut self) {
        if self.on_start {
            self.request_posters();
        } else {
            self.assign_media_rows();
            self.request_media_art();
        }
    }

    fn export_data(&self) -> ExportData {
        let (width, height) = self.export_size();
        let (num, den) = EXPORT_RATES[self.export.rate.min(2)];
        let rate = num as f32 / den as f32;
        let clips = self.timeline().clips.len();
        let titles = self
            .timeline()
            .clips
            .iter()
            .filter(|clip| clip.kind == model::ClipKind::Text)
            .count();
        ExportData {
            open: self.export.open,
            name: self.export.name.as_str().into(),
            path: format!(
                "{}/{}.mp4",
                self.export.folder.trim_end_matches('/'),
                self.export.name
            )
            .into(),
            format: format!("{width} × {height} · {rate:.2} fps").into(),
            duration: {
                let whole = self.duration().max(0.0) as i32;
                format!("{}:{:02}", whole / 60, whole % 60).into()
            },
            contents: if titles > 0 {
                format!("{clips} clips · {titles} titles")
            } else {
                format!("{clips} clips")
            }
            .into(),
            resolution: self.export.resolution as i32,
            rate: self.export.rate as i32,
            quality: self.export.quality as i32,
            size_high: bytes(self.export_size_bytes(0)).into(),
            size_balanced: bytes(self.export_size_bytes(1)).into(),
            size_small: bytes(self.export_size_bytes(2)).into(),
            phase: self.export.phase,
            progress: self.export.progress,
            stage: self.export.stage.as_str().into(),
            eta: if self.export.phase == ExportPhase::Running && self.export.progress > 0.02 {
                eta((1.0 - self.export.progress) * self.duration().max(1.0) * 2.0).into()
            } else {
                SharedString::new()
            },
            message: self.export.message.as_str().into(),
            done_size: bytes(self.export_size_bytes(self.export.quality)).into(),
            empty: clips == 0,
        }
    }

    fn model_rows(models: &[ModelState]) -> Vec<ModelData> {
        models
            .iter()
            .map(|model| {
                let total = model.megabytes;
                let fetched = model.fetched.unwrap_or(0.0);
                ModelData {
                    id: model.id.as_str().into(),
                    name: model.name.as_str().into(),
                    note: model.note.as_str().into(),
                    size: format!("{total:.0} MB").into(),
                    accuracy: model.accuracy,
                    installed: model.installed,
                    active: model.active && model.installed,
                    downloading: model.fetched.is_some(),
                    progress: if total > 0.0 {
                        (fetched / total).min(1.0)
                    } else {
                        0.0
                    },
                    transferred: if model.unpacking {
                        "Unpacking…".into()
                    } else {
                        format!("{fetched:.0} MB of {total:.0} MB").into()
                    },
                    eta: SharedString::new(),
                }
            })
            .collect()
    }

    /// The right-click menu for the clip it was opened on.
    fn menu(&self) -> Vec<MenuItemData> {
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
        let heading = |label: &str| MenuItemData {
            label: label.into(),
            kind: MenuRow::Label,
            ..Default::default()
        };
        let rule = || MenuItemData {
            kind: MenuRow::Separator,
            ..Default::default()
        };

        let mut rows = vec![
            heading(&clip.name),
            action("copy", "Copy".into(), Glyph::Copy, "⌘C", true),
            action("duplicate", "Duplicate".into(), Glyph::Plus, "⌘D", !locked),
            action(
                "paste",
                "Paste".into(),
                Glyph::Plus,
                "⌘V",
                self.clipboard.is_some(),
            ),
            rule(),
            action(
                "split",
                "Split at playhead".into(),
                Glyph::Split,
                "S",
                straddled && !locked,
            ),
            rule(),
        ];
        let audible = clip.kind != model::ClipKind::Image;
        rows.push(check(
            "mute",
            "Mute",
            "M",
            clip.volume <= 0.0,
            !locked && audible,
        ));
        rows.push(check("lock", "Lock track", "", locked, true));
        rows.push(rule());
        rows.push(MenuItemData {
            id: "delete".into(),
            label: "Delete".into(),
            kind: MenuRow::Action,
            glyph: Glyph::Trash,
            shortcut: "⌫".into(),
            enabled: !locked,
            danger: true,
            checkable: false,
            checked: false,
        });
        rows
    }

    pub fn menu_height(rows: &[MenuItemData]) -> f32 {
        let metrics = |kind: MenuRow| match kind {
            MenuRow::Action => 26.0,
            MenuRow::Label => 24.0,
            MenuRow::Separator => 9.0,
        };
        rows.iter().map(|row| metrics(row.kind)).sum::<f32>() + 8.0
    }

    /// The tray's A/V menu. What it offers depends on what is selected.
    fn av_menu(&self) -> Vec<MenuItemData> {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return Vec::new();
        };
        let row = |id: &str, label: &str, glyph: Glyph, enabled: bool| MenuItemData {
            id: id.into(),
            label: label.into(),
            kind: MenuRow::Action,
            glyph,
            shortcut: SharedString::new(),
            enabled,
            danger: false,
            checkable: false,
            checked: false,
        };
        if clip.kind == model::ClipKind::Text {
            return vec![row("speak", "Generate voice", Glyph::Volume, true)];
        }
        let has_sound = clip.kind != model::ClipKind::Image;
        let detached = self
            .timeline()
            .clips
            .iter()
            .any(|other| other.detached_from.as_deref() == Some(clip.id.as_str()));
        vec![
            row("captions", "Auto captions", Glyph::TextMark, has_sound),
            MenuItemData {
                kind: MenuRow::Separator,
                ..Default::default()
            },
            row(
                "detach",
                "Detach audio",
                Glyph::Waveform,
                clip.kind == model::ClipKind::Video && !detached,
            ),
            row(
                "reattach",
                "Reattach audio",
                Glyph::Merge,
                (clip.kind == model::ClipKind::Video && detached)
                    || (clip.kind == model::ClipKind::Audio && clip.detached_from.is_some()),
            ),
        ]
    }

    /// The File / Edit / View menus.
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
        let selected = self.selection.len();
        let playhead = f64::from(self.playhead);
        let straddled = self.timeline().clips.iter().any(|clip| {
            clip.start + f64::from(MIN_DURATION) < playhead
                && playhead < clip.start + clip.duration - f64::from(MIN_DURATION)
        });
        let (can_undo, can_redo) = self.session.as_ref().map_or((false, false), |session| {
            (session.can_undo(), session.can_redo())
        });
        let has_selection_media = !self.media_selected.is_empty();

        match self.open_menu {
            0 => vec![
                row(
                    "add-selected",
                    "Add selected to timeline".into(),
                    Glyph::Plus,
                    "",
                    has_selection_media,
                ),
                row("import", "Import media…".into(), Glyph::Import, "⌘I", true),
                row("save", "Save".into(), Glyph::Import, "⌘S", true),
                row(
                    "export",
                    "Export…".into(),
                    Glyph::Export,
                    "",
                    !self.timeline().clips.is_empty(),
                ),
                row(
                    "template",
                    "Save as template…".into(),
                    Glyph::Slot,
                    "",
                    true,
                ),
                row("speech", "Text to speech…".into(), Glyph::Volume, "", true),
                rule(),
                row("settings", "Settings…".into(), Glyph::Settings, "⌘,", true),
                rule(),
                row(
                    "close-project",
                    "Close project".into(),
                    Glyph::Import,
                    "",
                    true,
                ),
                MenuItemData {
                    id: "close-window".into(),
                    label: "Close window".into(),
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
                row("undo", "Undo".into(), Glyph::ChevronUp, "⌘Z", can_undo),
                row("redo", "Redo".into(), Glyph::ChevronDown, "⇧⌘Z", can_redo),
                rule(),
                row(
                    "split",
                    "Split at playhead".into(),
                    Glyph::Razor,
                    "⌘B",
                    straddled,
                ),
                MenuItemData {
                    id: "delete".into(),
                    label: if selected > 1 {
                        format!("Delete {selected} clips")
                    } else {
                        "Delete clip".into()
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
                rule(),
                MenuItemData {
                    id: "snap".into(),
                    label: "Snap to edges".into(),
                    kind: MenuRow::Action,
                    glyph: Glyph::None,
                    shortcut: "N".into(),
                    enabled: true,
                    danger: false,
                    checkable: true,
                    checked: self.snap,
                },
            ],
            2 => vec![
                row("zoom-in", "Zoom in".into(), Glyph::Plus, "+", true),
                row("zoom-out", "Zoom out".into(), Glyph::Minus, "-", true),
                rule(),
                row("start", "Go to start".into(), Glyph::SkipBack, "Home", true),
                row("end", "Go to end".into(), Glyph::SkipForward, "End", true),
            ],
            _ => Vec::new(),
        }
    }

    /// Everything the media bin's selection would add at the playhead.
    pub fn add_selected_media(&mut self) {
        let ids: Vec<String> = self
            .project()
            .media
            .iter()
            .filter(|item| self.media_selected.contains(&item.id))
            .map(|item| item.id.clone())
            .collect();
        let start = f64::from(self.playhead.max(0.0));
        for media_id in ids {
            self.apply(Command::AddClipAtFirstFree { media_id, start });
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
        let view = self.lane_view.entry(track.id.clone()).or_default();
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
            self.lane_view.entry(id).or_default().size = size;
        }
    }

    pub fn toggle_lock(&mut self, track_id: &str) {
        let view = self.lane_view.entry(track_id.to_owned()).or_default();
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

    /// Changes the project's output size from the monitor's picker.
    pub fn set_output(&mut self, index: usize) {
        let (width, height) = OUTPUTS[index.min(OUTPUTS.len() - 1)];
        if let Some(session) = self.session.as_mut() {
            session.prepare_save(None, Some(width as u32), Some(height as u32));
            self.dirty = true;
            self.schedule_autosave();
            self.request_preview();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Footprint, Studio};

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
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Concat's editor window, in Slint.
//!
//! Ported from the `slint/` tree of the wc-ui-rnd repository, where the UI was
//! built twice - once in React (its `web/` tree) and once here - so the two
//! stacks could be compared on one design. Comments that cite `web/` mean
//! that reference implementation; comments that cite `desktop/` mean the
//! Tauri editor this crate replaces, which stays in the repository until
//! this window can do everything it did.
//!
//! What is here is the window: every pane, dialog and primitive, driven by
//! demo data built in this file. Wiring it to `concat-project` and the rest
//! of the engine is the next step, not this one.

// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::{Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
// The drag payload type. Slint keeps `data-transfer` opaque in the language
// and hands it to the host to build, but the public Rust API has not caught up
// with the element yet — this is where the generated code itself reaches for
// it (`sp::DataTransfer`), so it is the same type the callbacks below expect.
use slint::private_unstable_api::re_exports::DataTransfer;

// Everything the .slint tree exports, in a module of its own: the workspace
// lints every public item for documentation, and the generated accessors are
// thousands of public items nobody documents. The allow covers them and
// nothing in this file.
#[allow(missing_docs)]
mod ui {
    slint::include_modules!();
}
use ui::*;

/// Effect names handed out as the demo adds rows, matching the web POOL.
const POOL: [&str; 6] = [
    "Sharpen",
    "Vignette",
    "Chroma Key",
    "Reverb",
    "Compressor",
    "De-esser",
];

/// x of a cubic bezier with endpoints pinned at 0 and 1, at parameter `t`.
fn bezier_axis(p1: f32, p2: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
}

fn bezier_axis_slope(p1: f32, p2: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * p1 + 6.0 * u * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

/// Solve a CSS cubic-bezier for y at a given x: Newton first, bisection as the
/// fallback where the curve is flat enough that Newton stalls. This is the
/// computation Slint's expression language cannot express — it has no loops —
/// so the timing function is evaluated here and read back through a global.
fn bezier_y_at_x(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    let mut t = x;

    for _ in 0..8 {
        let error = bezier_axis(x1, x2, t) - x;
        if error.abs() < 1e-5 {
            return bezier_axis(y1, y2, t);
        }
        let slope = bezier_axis_slope(x1, x2, t);
        if slope.abs() < 1e-6 {
            break;
        }
        t -= error / slope;
    }

    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    t = x;
    for _ in 0..24 {
        let at = bezier_axis(x1, x2, t);
        if (at - x).abs() < 1e-5 {
            break;
        }
        if at > x {
            hi = t;
        } else {
            lo = t;
        }
        t = (lo + hi) / 2.0;
    }
    bezier_axis(y1, y2, t)
}

/// "hh:mm:ss", "mm:ss" or "ss" -> seconds. Slint's string type has no split().
fn parse_timecode(text: &str) -> f32 {
    text.split(':')
        .rev()
        .enumerate()
        .map(|(index, part)| part.trim().parse::<f32>().unwrap_or(0.0) * 60_f32.powi(index as i32))
        .sum()
}

/// "hh:mm:ss:ff" -> frames. Short forms count from the right, so "12" is
/// twelve frames and "3:00" is three seconds — which is how anyone types into
/// a timecode field that is already showing them the shape.
fn parse_frames(text: &str, rate: f32) -> f32 {
    let fps = rate.round().max(1.0);
    let parts: Vec<f32> = text
        .split(':')
        .map(|part| part.trim().parse::<f32>().unwrap_or(0.0))
        .collect();
    let frames = parts.last().copied().unwrap_or(0.0);
    let seconds: f32 = parts
        .iter()
        .rev()
        .skip(1)
        .enumerate()
        .map(|(index, part)| part * 60_f32.powi(index as i32))
        .sum();
    (seconds * fps + frames).max(0.0)
}

/// "0.42, 0, 0.58, 1" -> Bezier, falling back to the current curve when the
/// text is not four numbers.
fn parse_bezier(text: &str, fallback: Bezier) -> Bezier {
    let parts: Vec<f32> = text
        .split(',')
        .filter_map(|part| part.trim().parse::<f32>().ok())
        .collect();
    match parts[..] {
        [x1, y1, x2, y2] => Bezier { x1, y1, x2, y2 },
        _ => fallback,
    }
}

/// The bin, held here rather than in Slint.
///
/// Slint's expression language cannot filter a model, and filtering inside the
/// panel's `for` would leave holes in the grid's indices — the cards are
/// placed by arithmetic on the row number, so row 4 being invisible would
/// leave a gap where a card should be. So the whole library lives here and the
/// panel is handed exactly the rows it should draw, plus the counts of the
/// rows it should not.
struct Library {
    items: RefCell<Vec<MediaItemData>>,
    filter: Cell<MediaFilter>,
}

impl Library {
    fn new(items: Vec<MediaItemData>) -> Self {
        Self {
            items: RefCell::new(items),
            filter: Cell::new(MediaFilter::All),
        }
    }

    fn shows(filter: MediaFilter, kind: MediaKind) -> bool {
        match filter {
            MediaFilter::All => true,
            MediaFilter::Video => kind == MediaKind::Video,
            MediaFilter::Audio => kind == MediaKind::Audio,
            MediaFilter::Images => kind == MediaKind::Image,
        }
    }

    /// One row of the bin, by the id a drag payload carries.
    fn item(&self, id: i32) -> Option<MediaItemData> {
        self.items
            .borrow()
            .iter()
            .find(|item| item.id == id)
            .cloned()
    }

    fn count(&self, kind: MediaKind) -> i32 {
        self.items
            .borrow()
            .iter()
            .filter(|item| item.kind == kind)
            .count() as i32
    }

    /// Republishes the filtered view and every count the sidebar reads. Called
    /// after anything at all changes: the bin is tens of rows, not thousands,
    /// and one path that always produces a consistent panel is worth more than
    /// the row-level updates it could be split into.
    fn publish(&self, app: &App, view: &VecModel<MediaItemData>) {
        let filter = self.filter.get();
        let items = self.items.borrow();

        // Row-wise, like the timeline's: selecting a card must not rebuild the
        // grid under the pointer that selected it. The envelopes came with the
        // rows — nothing the bin does can change one.
        sync(
            view,
            items
                .iter()
                .filter(|item| Self::shows(filter, item.kind))
                .cloned()
                .collect(),
        );

        let editor = app.global::<Editor>();
        editor.set_media_count_all(items.len() as i32);
        editor.set_media_count_video(self.count(MediaKind::Video));
        editor.set_media_count_audio(self.count(MediaKind::Audio));
        editor.set_media_count_images(self.count(MediaKind::Image));
        editor.set_media_selected_count(items.iter().filter(|item| item.selected).count() as i32);
    }
}

fn media(id: i32, name: &str, kind: MediaKind, duration: f32) -> MediaItemData {
    MediaItemData {
        id,
        name: SharedString::from(name),
        kind,
        duration,
        // The same envelope, from the same seed, as the clip cut from this
        // file: `m{id}` is what the demo timeline's clips name as their media,
        // so a file looks like itself in the bin and on a lane. Built once
        // with the row, because it depends on nothing the bin can change.
        wave: if kind == MediaKind::Audio {
            wave_path(&format!("m{id}"), 0.0, duration, 1.0).into()
        } else {
            SharedString::new()
        },
        // No decoder in this tree yet, so no artwork: the card falls back to
        // the kind's tint and mark, which is what it does in the reference
        // until the filmstrip or the peaks arrive.
        ..Default::default()
    }
}

/// A bin the size of a real one, so the grid is exercised at the widths a
/// splitter drag actually produces.
fn demo_library() -> Vec<MediaItemData> {
    use MediaKind::{Audio, Video};
    vec![
        media(1, "ElevenLabs_2026-08-16T09-12-04_intro.mp3", Audio, 3.4),
        media(2, "0001-0036.mp4", Video, 1.2),
        media(3, "0036-0072.mp4", Video, 1.2),
        media(4, "0072-0134.mp4", Video, 2.1),
        media(5, "0134-0182.mp4", Video, 1.6),
        media(6, "0232-0322.mp4", Video, 3.0),
        media(7, "0326-0452.mp4", Video, 4.2),
        media(8, "0456-0524.mp4", Video, 2.3),
        media(9, "0528-0604.mp4", Video, 2.5),
        media(10, "0604-0642.mp4", Video, 1.3),
        media(11, "0654-0725.mp4", Video, 2.4),
        media(12, "0730-0781.mp4", Video, 1.7),
        media(13, "ElevenLabs_2026-08-16T09-14-22_line-02.mp3", Audio, 1.9),
        media(14, "ElevenLabs_2026-08-16T09-15-40_line-03.mp3", Audio, 2.6),
        media(15, "ElevenLabs_2026-08-16T09-17-01_line-04.mp3", Audio, 7.8),
        media(16, "ElevenLabs_2026-08-16T09-18-33_line-05.mp3", Audio, 4.5),
        media(17, "ElevenLabs_2026-08-16T09-20-09_outro.mp3", Audio, 5.2),
    ]
}

/// Programme level for the meter: fast attack, slow release, with a peak hold
/// that decays after 900ms — the behaviour the web demo fakes in a rAF loop.
struct LevelSim {
    seed: Cell<u32>,
    current: Cell<f32>,
    held: Cell<f32>,
    hold_frames: Cell<u32>,
    elapsed: Cell<f32>,
}

impl LevelSim {
    fn new() -> Self {
        Self {
            seed: Cell::new(0x2545_f491),
            current: Cell::new(0.0),
            held: Cell::new(0.0),
            hold_frames: Cell::new(0),
            elapsed: Cell::new(0.0),
        }
    }

    fn random(&self) -> f32 {
        // xorshift32, so the demo needs no rand dependency
        let mut x = self.seed.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed.set(x);
        (x >> 8) as f32 / (1 << 24) as f32
    }

    /// Advances one tick and returns (level, peak).
    fn tick(&self, dt: f32) -> (f32, f32) {
        self.elapsed.set(self.elapsed.get() + dt);
        let target = 0.45 + (self.elapsed.get() * 2.4).sin() * 0.2 + self.random() * 0.25;
        let current = self.current.get();
        let next = if target > current {
            target
        } else {
            current + (target - current) * 0.08
        };
        self.current.set(next);

        let frames = self.hold_frames.get();
        if next > self.held.get() || frames == 0 {
            self.held.set(next);
            self.hold_frames.set(27); // ~900ms at 33ms per tick
        } else {
            self.hold_frames.set(frames - 1);
        }
        (next, self.held.get())
    }

    fn reset(&self) {
        self.current.set(0.0);
        self.held.set(0.0);
        self.hold_frames.set(0);
    }
}

// ─── the timeline ───────────────────────────────────────────────────────────
//
// The project, held here for the reason the bin is: a chain is edited by index
// and a Slint model cannot be spliced from the language. Slint is handed a
// flattened view — clips already carrying the row they draw on — and every
// mutation ends in the same `publish`, so there is one place where the panel's
// idea of the timeline can go wrong.

/// The ruler's tick ladder. The smallest interval still at least 90px apart at
/// this zoom, which is a table search — a loop, and so not something Slint's
/// expression language can express.
const TICKS: [f32; 16] = [
    1.0 / 30.0,
    0.1,
    0.25,
    0.5,
    1.0,
    2.0,
    5.0,
    10.0,
    15.0,
    30.0,
    60.0,
    120.0,
    300.0,
    600.0,
    1800.0,
    3600.0,
];

fn tick_interval(seconds_per_pixel: f32) -> f32 {
    TICKS
        .iter()
        .copied()
        .find(|interval| interval / seconds_per_pixel >= 90.0)
        .unwrap_or(3600.0)
}

// ─── the dialogs ────────────────────────────────────────────────────────────

/// Export output sizes, matching the picker's rows.
const EXPORT_SIZES: [(i32, i32); 4] = [(3840, 2160), (2560, 1440), (1920, 1080), (1280, 720)];
const EXPORT_RATES: [f32; 3] = [24.0, 30.0, 60.0];
/// Video bitrate in Mbps at 1080p30, scaled by pixel count and frame rate —
/// the model web/'s export panel uses, because the number it produces is the
/// one the choice is actually made on.
const EXPORT_TIERS: [f32; 3] = [16.0, 8.0, 4.0];
const AUDIO_BPS: f32 = 192_000.0;

struct ExportState {
    open: bool,
    name: String,
    folder: String,
    resolution: usize,
    rate: usize,
    quality: usize,
    phase: ExportPhase,
    progress: f32,
    message: String,
}

impl Default for ExportState {
    fn default() -> Self {
        Self {
            open: false,
            name: "Untitled project".into(),
            folder: "~/Movies/Concat".into(),
            // 1080p30 balanced: the middle of every list, which is what a
            // default should be when the project has not said otherwise.
            resolution: 2,
            rate: 1,
            quality: 1,
            phase: ExportPhase::Idle,
            progress: 0.0,
            message: String::new(),
        }
    }
}

#[derive(Default)]
struct SettingsState {
    open: bool,
    tab: i32,
    language: usize,
    transcribe_language: i32,
}

/// The bottom-right notice — one at a time, because a column of them is a log
/// and what this is for is the one sentence about what just happened.
///
/// The token is what the panel watches. It is bumped per notice rather than
/// the message being compared, so the same sentence said twice is two
/// notices: saving twice should blink twice, and a message-equality test
/// would show the first and swallow the second.
#[derive(Default)]
struct ToastState {
    token: i32,
    message: String,
    failed: bool,
}

#[derive(Clone)]
struct ModelState {
    id: String,
    name: String,
    note: String,
    /// on disk, in megabytes
    megabytes: f32,
    accuracy: i32,
    installed: bool,
    active: bool,
    /// megabytes fetched so far, while a download is in flight
    fetched: Option<f32>,
}

fn demo_transcribers() -> Vec<ModelState> {
    vec![
        ModelState {
            id: "tiny".into(),
            name: "Tiny".into(),
            note: "Fastest, roughest".into(),
            megabytes: 78.0,
            accuracy: 1,
            installed: true,
            active: false,
            fetched: None,
        },
        ModelState {
            id: "base".into(),
            name: "Base".into(),
            note: "Quick, usable for notes".into(),
            megabytes: 148.0,
            accuracy: 2,
            installed: true,
            active: true,
            fetched: None,
        },
        ModelState {
            id: "small".into(),
            name: "Small".into(),
            note: "The usual choice".into(),
            megabytes: 488.0,
            accuracy: 3,
            installed: false,
            active: false,
            fetched: None,
        },
        ModelState {
            id: "medium".into(),
            name: "Medium".into(),
            note: "Slower, noticeably better".into(),
            megabytes: 1530.0,
            accuracy: 4,
            installed: false,
            active: false,
            fetched: None,
        },
        ModelState {
            id: "large".into(),
            name: "Large v3".into(),
            note: "Best, and it will cost you".into(),
            megabytes: 3090.0,
            accuracy: 5,
            installed: false,
            active: false,
            fetched: None,
        },
    ]
}

fn demo_voices() -> Vec<ModelState> {
    vec![
        ModelState {
            id: "vc-en".into(),
            name: "English · Neutral".into(),
            note: "Two speakers, 22kHz".into(),
            megabytes: 63.0,
            accuracy: 3,
            installed: true,
            active: true,
            fetched: None,
        },
        ModelState {
            id: "vc-en-hq".into(),
            name: "English · Studio".into(),
            note: "One speaker, 48kHz".into(),
            megabytes: 214.0,
            accuracy: 5,
            installed: false,
            active: false,
            fetched: None,
        },
        ModelState {
            id: "vc-zh".into(),
            name: "Chinese · Neutral".into(),
            note: "Two speakers, 22kHz".into(),
            megabytes: 71.0,
            accuracy: 3,
            installed: false,
            active: false,
            fetched: None,
        },
    ]
}

/// Bytes in the units a person reads them in.
fn bytes(count: f32) -> String {
    if count >= 1_000_000_000.0 {
        format!("{:.1} GB", count / 1_000_000_000.0)
    } else if count >= 1_000_000.0 {
        format!("{:.0} MB", count / 1_000_000.0)
    } else {
        format!("{:.0} KB", (count / 1_000.0).max(1.0))
    }
}

/// Seconds as a rough remaining time. Rough on purpose: a countdown to the
/// second on an estimate that is not accurate to the second is theatre.
fn eta(seconds: f32) -> String {
    if seconds <= 1.0 {
        "almost done".into()
    } else if seconds < 60.0 {
        format!("{:.0}s left", seconds)
    } else {
        format!(
            "{:.0}m {:02.0}s left",
            (seconds / 60.0).floor(),
            seconds % 60.0
        )
    }
}

/// The monitor's output sizes, matching the picker's rows.
const OUTPUTS: [(i32, i32); 6] = [
    (1920, 1080),
    (3840, 2160),
    (1080, 1920),
    (1080, 1080),
    (1440, 1080),
    (2560, 1080),
];

/// Shortest clip the editor will make: a sixtieth of a second, as the
/// reference's model puts it. Trims and splits both floor at this.
const MIN_DURATION: f32 = 1.0 / 60.0;

/// The three lane heights, in logical pixels.
///
/// Picture gets the tallest because a filmstrip is the one thing that needs
/// the room; sound the middle, where an envelope still has shape; a filter
/// layer the least, because it is a bar with a name on it and nothing else to
/// show. The ladder is what `TrackSize::Auto` picks from — see `lane_height`.
const LANE_LARGE: f32 = 80.0;
const LANE_MEDIUM: f32 = 60.0;
const LANE_SMALL: f32 = 40.0;

/// How long a title or a filter layer runs when it is placed: long enough to
/// read, short enough that trimming it is a nudge rather than a fight. The
/// same three seconds the reference gives a new title.
const LAYER_DURATION: f32 = 3.0;

/// Steps per second the drawn waveform is quantised to. See `Studio::wave`:
/// it is what keeps a trim from synthesising a new envelope on every pointer
/// event, and the grid is fine enough that no step of it is visible.
const WAVE_STEPS: f32 = 30.0;

#[derive(Clone)]
struct ClipDoc {
    id: String,
    name: String,
    kind: ClipKind,
    /// Track id, not a row: rows are a fact about the view, and a clip that
    /// stored one would be wrong the moment a lane above it was removed.
    track: String,
    /// The file behind it, for the peaks. Empty for a title and for a filter.
    media: String,
    /// The catalogue id a filter layer was made from — "telephone", "hall".
    /// Empty for everything else. The label the lane draws is `name`; this is
    /// what an engine would be handed, and what a project file would store.
    preset: String,
    start: f32,
    duration: f32,
    /// In-point: how far into the media the clip starts.
    source_start: f32,
    /// Multiplier over the fitted size; 1 fills the frame, preserving aspect.
    scale: f32,
    /// Offset from centred, as a fraction of frame width / height.
    offset_x: f32,
    offset_y: f32,
    /// Clockwise degrees about the picture's centre.
    rotation: f32,
    opacity: f32,
    /// Playback rate; 1 is normal.
    speed: f32,
    /// Keep voices at their natural pitch when `speed` is not 1. On by
    /// default; off gives the tape-machine sound.
    preserve_pitch: bool,
    volume: f32,
    fade_in: f32,
    fade_out: f32,
    fx: bool,
    transition_in: bool,
    text_body: String,
    /// On a detached audio clip: the video clip the sound came from.
    detached_from: Option<String>,
    /// The overlay, when this is a text clip. Every field is a fraction of the
    /// output frame rather than an absolute size: the frame can be any
    /// resolution, and a title dialled in at 1080p should not move at 4K.
    text: TextStyleDoc,
}

#[derive(Clone)]
struct TextStyleDoc {
    family: String,
    /// share of frame height
    size: f32,
    weight: f32,
    italic: bool,
    fill: slint::Color,
    align: TextAlignment,
    opacity: f32,
    stroke_width: f32,
    stroke: slint::Color,
    shadow: bool,
    /// The plate behind the words. Fully transparent is how "no plate" is
    /// stored — it is a colour, not a flag, so turning it off is clearing it.
    plate: slint::Color,
    line_height: f32,
    tracking: f32,
}

impl Default for TextStyleDoc {
    fn default() -> Self {
        Self {
            family: "Inter".into(),
            size: 0.09,
            weight: 600.0,
            italic: false,
            fill: slint::Color::from_rgb_u8(0xff, 0xff, 0xff),
            align: TextAlignment::Center,
            opacity: 1.0,
            stroke_width: 0.0,
            stroke: slint::Color::from_rgb_u8(0x00, 0x00, 0x00),
            plate: slint::Color::from_argb_u8(0, 0, 0, 0),
            shadow: true,
            line_height: 1.2,
            tracking: 0.0,
        }
    }
}

/// "#rrggbb", for the readout beside a swatch — Slint cannot print a colour.
fn hex_of(colour: slint::Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        colour.red(),
        colour.green(),
        colour.blue()
    )
}

impl ClipDoc {
    fn end(&self) -> f32 {
        self.start + self.duration
    }
}

#[derive(Clone)]
struct TrackDoc {
    id: String,
    name: String,
    visible: bool,
    muted: bool,
    /// Nothing on this lane can be selected, moved, trimmed or cut, and
    /// nothing can be dropped onto it.
    locked: bool,
    /// How tall to draw it. `Auto` — the default — takes the height from what
    /// is on the lane; see `lane_height`.
    size: TrackSize,
}

struct TimelineDoc {
    id: String,
    name: String,
    /// Bottom-most lane first, matching the reference — that is the engine's
    /// compositing order. The panel draws the top lane first, so every place
    /// that speaks in rows goes through `row_track`.
    tracks: Vec<TrackDoc>,
    clips: Vec<ClipDoc>,
}

/// Which edge of a clip a trim has hold of.
#[derive(Clone, Copy, PartialEq)]
enum Edge {
    Start,
    End,
}

/// Where one clip sat when a gesture began, so the whole set moves rigidly.
struct MoveOrigin {
    clip: String,
    start: f32,
    row: i32,
}

enum Gesture {
    None,
    Move {
        /// The clip actually grabbed; it is the one that snaps.
        primary: String,
        origins: Vec<MoveOrigin>,
        /// The lane heights as they were when the press landed.
        ///
        /// Frozen, not read live, because an `Auto` lane takes its height from
        /// what is on it — so the act of dragging a clip onto a lane resizes
        /// that lane and the ones under it, which moves the very edges the
        /// next event measures itself against. Live, a drag of the wrong
        /// length lands the clip on a lane that shrinks the lane it came
        /// from, which puts the same pointer back over the first lane, and the
        /// clip flips between the two for as long as the mouse is held still.
        lanes: Vec<f32>,
    },
    Trim {
        clip: String,
        edge: Edge,
        start: f32,
        duration: f32,
        source_start: f32,
    },
}

struct Studio {
    /// Envelopes, keyed by the four things they are computed from. A move
    /// changes none of them, and a publish happens on every frame of one —
    /// without this, dragging a clip rebuilds a few kilobytes of path string
    /// per audio clip per frame for a shape that did not change.
    waves: RefCell<std::collections::HashMap<String, SharedString>>,
    timelines: Vec<TimelineDoc>,
    active: usize,
    selection: Vec<String>,
    playhead: f32,
    scroll_left: f32,
    seconds_per_pixel: f32,
    frame_rate: f32,
    tool: TimelineTool,
    snap: bool,
    /// Index into the monitor's output list, and into its quality list. The
    /// output is a property of the project rather than of the view — it is
    /// what gets exported — but nothing else reads it yet.
    ratio: usize,
    /// 0 Full, 1 Half, 2 Quarter. Half by default, as the reference opens.
    quality: usize,
    playing: bool,
    /// One clip, held for Paste. A real clipboard is the system's and holds a
    /// serialised edit; this is the seam that becomes one.
    clipboard: Option<ClipDoc>,
    /// The two sheets. Held with the rest of the state because what they show
    /// is the project, and because only one of them can be up at a time.
    export: ExportState,
    settings: SettingsState,
    transcribers: Vec<ModelState>,
    voices: Vec<ModelState>,
    /// The A/V tray menu, built like the clip menu and shown the same way.
    av_token: i32,
    /// Which title-bar menu is open, or -1, and the token that shows it.
    open_menu: i32,
    menu_bar_token: i32,
    /// The clip the context menu was opened on. Held separately from the
    /// selection because the menu outlives the press that opened it.
    menu_target: Option<String>,
    menu_token: i32,
    /// The launch screen: whether it is up, the form on it, and the projects
    /// it offers to reopen. Held with the rest because what it makes is the
    /// project — the frame rate it chooses is the timeline's frame rate, and
    /// the name it chooses is what the title bar says for the rest of the run.
    on_start: bool,
    start: StartState,
    recents: Vec<RecentProject>,
    project_name: String,

    /// The workspace's arrangement, and the box it is arranged in. The box
    /// comes up from Slint — see `Editor.workspace-resized`; it is the one
    /// thing about the layout this side cannot work out for itself.
    dock: Dock,
    workspace: (f32, f32),
    /// The gutter a pointer has hold of: which split, the ratio it was at when
    /// the press landed, and the extent that ratio is a fraction of. A drag is
    /// one pointer, so one slot is enough.
    divider_press: Option<(usize, f32, f32)>,
    gesture: Gesture,
    /// The drag from the library that is over the lanes, already resolved
    /// into the clip it would leave. None when nothing is hovering.
    drop: Option<DropPlan>,
    /// The last notice raised, and the count that makes it new. Held with the
    /// rest of the state because every publish carries it, and because what
    /// raises one is whatever else the same handler just did.
    toast: ToastState,
    next_id: u32,
}

/// A drag from the library, resolved against the timeline.
///
/// One struct for both halves of the gesture, because the ghost the lanes draw
/// and the clip the release makes have to be the same thing — the hover
/// answers "what would this be", and the drop is that answer, committed.
#[derive(Clone)]
struct DropPlan {
    kind: ClipKind,
    /// What the clip will be called, and what the ghost's header says.
    label: String,
    /// The catalogue id, for a filter.
    preset: String,
    /// The bin's media id, for a clip cut from a file.
    media: String,
    start: f32,
    duration: f32,
    row: i32,
}

/// Every model the window is handed, kept for its lifetime rather than
/// rebuilt.
///
/// A new model, or `set_vec` on the one already there, is a *reset* — and
/// Slint answers a reset by dropping every instance the repeater behind it
/// built and starting again. Mid-gesture that is fatal: the TouchArea holding
/// the pointer is one of the instances dropped, so a trim moved one notch and
/// then let go of the mouse. `sync` below is what replaced it.
struct Models {
    tabs: Rc<VecModel<TimelineTabData>>,
    tracks: Rc<VecModel<TrackData>>,
    clips: Rc<VecModel<ClipData>>,
    /// The clip's context menu, the A/V tray's, and the title bar's.
    menu: Rc<VecModel<MenuItemData>>,
    av: Rc<VecModel<MenuItemData>>,
    bar: Rc<VecModel<MenuItemData>>,
    /// The two engine lists in Settings.
    transcribers: Rc<VecModel<ModelData>>,
    voices: Rc<VecModel<ModelData>>,
    /// The workspace's arrangement, walked flat.
    seats: Rc<VecModel<SeatBox>>,
    dividers: Rc<VecModel<DockDivider>>,
    /// The launch screen's list of projects to go back to.
    recents: Rc<VecModel<RecentProjectData>>,
}

/// Republish a list into a live model without resetting it.
///
/// Rows that did not change are not written at all, and a write is a
/// `row-changed` on an instance that stays alive: no teardown, so a gesture
/// survives the republish it caused, and no re-layout or re-parse of a
/// waveform's path for a clip the pointer never went near.
fn sync<T: Clone + PartialEq + 'static>(model: &VecModel<T>, next: Vec<T>) {
    let mut next = next;
    let shared = model.row_count().min(next.len());
    let tail = next.split_off(shared);
    for (row, value) in next.into_iter().enumerate() {
        // Compared before writing: `set-row-data` notifies whether or not the
        // row moved, and a notification costs the row a re-layout.
        if model.row_data(row).as_ref() != Some(&value) {
            model.set_row_data(row, value);
        }
    }
    // Then the difference in length, a row at a time: `row-added` and
    // `row-removed` shift the instances either side of the change rather than
    // rebuilding them, which is the one thing a reset cannot do.
    while model.row_count() > shared {
        model.remove(model.row_count() - 1);
    }
    for value in tail {
        model.push(value);
    }
}

impl Studio {
    fn now(&self) -> &TimelineDoc {
        &self.timelines[self.active]
    }

    fn now_mut(&mut self) -> &mut TimelineDoc {
        &mut self.timelines[self.active]
    }

    /// The track a row index names. Rows count from the top of the panel and
    /// the model stores lanes from the bottom, so this is the one place the
    /// two orders meet.
    fn row_track(&self, row: i32) -> Option<&TrackDoc> {
        let tracks = &self.now().tracks;
        let count = tracks.len() as i32;
        if row < 0 || row >= count {
            return None;
        }
        tracks.get((count - 1 - row) as usize)
    }

    fn row_of(&self, track_id: &str) -> i32 {
        let tracks = &self.now().tracks;
        tracks
            .iter()
            .position(|track| track.id == track_id)
            .map(|index| tracks.len() as i32 - 1 - index as i32)
            .unwrap_or(0)
    }

    fn clip(&self, id: &str) -> Option<&ClipDoc> {
        self.now().clips.iter().find(|clip| clip.id == id)
    }

    fn mint(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    /// The next lane's name. Numbered from the highest already in use and not
    /// from the count, which is the engine's rule: adding a lane after
    /// removing one should not hand out a name the project has seen before.
    fn next_track_name(&self) -> String {
        let highest = self
            .now()
            .tracks
            .iter()
            .filter_map(|track| track.name.rsplit(' ').next()?.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        format!("Track {}", highest + 1)
    }

    /// Whether a lane refuses to be edited.
    fn locked(&self, track_id: &str) -> bool {
        self.now()
            .tracks
            .iter()
            .any(|track| track.id == track_id && track.locked)
    }

    /// How tall one lane is drawn.
    ///
    /// The auto case is the whole point of the ladder: a lane takes the height
    /// of the tallest thing on it, so a lane of footage opens large, a lane of
    /// sound medium and a lane of filter layers minimal — without the lanes
    /// having to be typed, which they deliberately are not. An empty one takes
    /// the middle size; it has nothing to be told by, and starting small would
    /// make a new lane look like a place nothing belongs.
    fn lane_height(&self, lane: &TrackDoc) -> f32 {
        match lane.size {
            TrackSize::Small => LANE_SMALL,
            TrackSize::Medium => LANE_MEDIUM,
            TrackSize::Large => LANE_LARGE,
            TrackSize::Auto => {
                let tallest = self
                    .now()
                    .clips
                    .iter()
                    .filter(|clip| clip.track == lane.id)
                    .map(|clip| match clip.kind {
                        ClipKind::Video | ClipKind::Image => LANE_LARGE,
                        ClipKind::Audio | ClipKind::Text => LANE_MEDIUM,
                        ClipKind::Filter => LANE_SMALL,
                    })
                    .fold(0.0_f32, f32::max);
                if tallest > 0.0 { tallest } else { LANE_MEDIUM }
            }
        }
    }

    /// Every lane's height, top-most first — the order the panel draws them
    /// and the order `row_of` counts in.
    ///
    /// Rebuilt on demand rather than cached: it is a handful of lanes over a
    /// handful of clips, and a cache would be one more thing that can disagree
    /// with the document.
    fn lane_heights(&self) -> Vec<f32> {
        self.now()
            .tracks
            .iter()
            .rev()
            .map(|lane| self.lane_height(lane))
            .collect()
    }

    /// The row a point down the stack falls in, clamped to the stack: past the
    /// foot is the last lane, not nothing.
    fn row_at(&self, y: f32) -> i32 {
        row_at(&self.lane_heights(), y)
    }

    /// Seconds the project runs to, for Fit and for the scroll floor.
    fn duration(&self) -> f32 {
        self.now()
            .clips
            .iter()
            .fold(0.0_f32, |longest, clip| longest.max(clip.end()))
    }

    /// The reference's snapTime: clip edges, the playhead and zero all pull,
    /// and the nearest inside the threshold wins.
    fn snapped(&self, time: f32, threshold: f32, exclude: &str) -> f32 {
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
        for clip in &self.now().clips {
            if clip.id == exclude {
                continue;
            }
            consider(clip.start);
            consider(clip.end());
        }
        best
    }

    /// What a library payload names — "media:12", "text:default:Title",
    /// "filter:telephone:Telephone" — before the timeline has decided where to
    /// put it.
    ///
    /// `start` and `row` come back zeroed: a drop takes both from the pointer,
    /// and a click has to choose them, so neither can be settled here. None is
    /// a payload this tree cannot read — a drag from another application,
    /// which is also how one is refused.
    fn incoming(payload: &str, library: &Library) -> Option<DropPlan> {
        let mut fields = payload.splitn(3, ':');
        let (sort, id) = (fields.next()?, fields.next()?);
        let label = fields.next().unwrap_or(id);

        let (kind, label, preset, media, duration) = match sort {
            "media" => {
                let item = library.item(id.parse().ok()?)?;
                (
                    match item.kind {
                        MediaKind::Audio => ClipKind::Audio,
                        MediaKind::Image => ClipKind::Image,
                        _ => ClipKind::Video,
                    },
                    item.name.to_string(),
                    String::new(),
                    format!("m{}", item.id),
                    // A still has no length of its own — it runs for as long
                    // as it is given, which is the same span a title gets.
                    if item.kind == MediaKind::Image {
                        LAYER_DURATION
                    } else {
                        item.duration
                    },
                )
            }
            "text" => (
                ClipKind::Text,
                label.to_string(),
                String::new(),
                String::new(),
                LAYER_DURATION,
            ),
            "filter" => (
                ClipKind::Filter,
                label.to_string(),
                id.to_string(),
                String::new(),
                LAYER_DURATION,
            ),
            _ => return None,
        };

        Some(DropPlan {
            kind,
            label,
            preset,
            media,
            start: 0.0,
            duration: duration.max(MIN_DURATION),
            row: 0,
        })
    }

    /// A drag over the lanes: the pointer names the moment and the lane.
    ///
    /// None is a refusal, and the drag is told — the lanes turn it into
    /// `DragAction.none`, which is what puts up the no-drop cursor.
    fn plan(&self, payload: &str, seconds: f32, row: i32, library: &Library) -> Option<DropPlan> {
        let mut plan = Studio::incoming(payload, library)?;

        let lanes = self.now().tracks.len() as i32;
        if lanes == 0 {
            return None;
        }
        // Past the foot of the stack is the last lane, not nothing: the lanes
        // are drawn taller than the tracks on them, and a drop in that empty
        // band means the lane above it.
        plan.row = row.clamp(0, lanes - 1);
        // A locked lane takes no drops, the same way it takes no presses.
        if self.row_track(plan.row).is_none_or(|track| track.locked) {
            return None;
        }
        // The same snap the gestures use pulls the start onto whatever edge is
        // nearby — so a filter laid over a clip can be laid over exactly it.
        plan.start = self
            .snapped(seconds.max(0.0), 8.0 * self.seconds_per_pixel, "")
            .max(0.0);
        Some(plan)
    }

    /// A card clicked rather than dragged: no pointer over the lanes, so the
    /// playhead names the moment and the topmost lane with room for it — which
    /// is a question only answerable once the length is known, which is why
    /// the row is settled here and not in `incoming`.
    fn plan_at_playhead(&mut self, payload: &str, library: &Library) -> Option<DropPlan> {
        let mut plan = Studio::incoming(payload, library)?;
        plan.start = self.playhead.max(0.0);
        plan.row = self.free_row(plan.start, plan.duration);
        Some(plan)
    }

    /// Commit a plan, and select what it made.
    fn place(&mut self, plan: &DropPlan) -> Option<String> {
        let track = self.row_track(plan.row)?.id.clone();
        let id = self.mint("c");
        let mut placed = clip(
            &id,
            &plan.label,
            plan.kind,
            &track,
            &plan.media,
            plan.start,
            plan.duration,
        );
        placed.preset = plan.preset.clone();
        if plan.kind == ClipKind::Text {
            // Something to see on the lane and to type over in the inspector.
            // An empty title reads as a broken one.
            placed.text_body = "New title".into();
        }
        self.now_mut().clips.push(placed);
        // Selected, so the inspector opens on what was just placed rather than
        // on whatever happened to be selected before it.
        self.selection = vec![id.clone()];
        Some(id)
    }

    /// The topmost lane with room for a clip of this length at this moment,
    /// adding one above the stack when every lane is busy.
    ///
    /// Only the click paths need this. A drop has a pointer, and a pointer is
    /// a better answer than any rule.
    fn free_row(&mut self, start: f32, duration: f32) -> i32 {
        let lanes = self.now().tracks.len() as i32;
        for row in 0..lanes {
            let Some(track) = self.row_track(row) else {
                continue;
            };
            if track.locked {
                continue;
            }
            let lane = track.id.clone();
            let busy = self.now().clips.iter().any(|clip| {
                clip.track == lane && clip.start < start + duration && start < clip.end()
            });
            if !busy {
                return row;
            }
        }
        let id = self.mint("T");
        let name = self.next_track_name();
        // Pushed, not inserted: the model runs bottom-up, so the end of the
        // list is the top of the stack — which is row zero.
        self.now_mut().tracks.push(track(&id, &name));
        0
    }

    /// Why the selection cannot be merged, or None when it can. The reason is
    /// carried rather than a bare bool: a button that greys out without saying
    /// why leaves you guessing at the rule.
    fn merge_blocked(&self) -> Option<&'static str> {
        if self.selection.len() != 2 {
            return Some("Select two clips to merge");
        }
        let (Some(a), Some(b)) = (self.clip(&self.selection[0]), self.clip(&self.selection[1]))
        else {
            return Some("Select two clips to merge");
        };
        let (first, second) = if a.start <= b.start { (a, b) } else { (b, a) };
        if first.track != second.track {
            return Some("Both clips must be on one track");
        }
        if first.media != second.media || first.media.is_empty() {
            return Some("Both clips must come from one file");
        }
        if (second.start - first.end()).abs() > MIN_DURATION {
            return Some("The two clips must be touching");
        }
        if (second.source_start - (first.source_start + first.duration)).abs() > MIN_DURATION {
            return Some("The cut has material missing between them");
        }
        None
    }

    /// Sound to detach, or a title's words to speak — the reference only hangs
    /// the A/V menu in the tray when the selection has something to offer it.
    fn has_av_tools(&self) -> bool {
        self.selection.len() == 1
            && self
                .clip(&self.selection[0])
                .is_some_and(|clip| !matches!(clip.kind, ClipKind::Image | ClipKind::Filter))
    }

    /// The memoised envelope for one clip.
    fn wave(&self, clip: &ClipDoc) -> SharedString {
        // Quantised, not just rounded. A trim moves the ends by a fraction of
        // a frame per pointer event, and at a millisecond of resolution every
        // one of those was a fresh 128-column path: built here, re-parsed by
        // the renderer, and kept in this map for the life of the process. A
        // thirtieth of a second is finer than the difference can be seen once
        // the shape is stretched onto the clip, and it bounds the map to the
        // handful of entries a drag actually visits.
        let step = |seconds: f32| (seconds * WAVE_STEPS).round() / WAVE_STEPS;
        let (source_start, duration) = (step(clip.source_start), step(clip.duration));
        let key = format!(
            "{}|{:.3}|{:.3}|{:.3}",
            clip.media, source_start, duration, clip.volume
        );
        if let Some(cached) = self.waves.borrow().get(&key) {
            return cached.clone();
        }
        // Cached as the string Slint itself holds, not as a Rust one: a
        // `SharedString` clone is a refcount, so handing the same envelope to
        // the model on every event of a drag copies nothing.
        let built = SharedString::from(wave_path(&clip.media, source_start, duration, clip.volume));
        self.waves.borrow_mut().insert(key, built.clone());
        built
    }

    /// The selection, flattened for the inspector.
    ///
    /// Exactly one clip or nothing: a panel of numbers describing two clips at
    /// once can only lie, so a multiple selection reads as no selection —
    /// which is what puts the pane back on the project's details.
    fn selected(&self) -> SelectedClipData {
        let Some(clip) = self
            .selection
            .first()
            .filter(|_| self.selection.len() == 1)
            .and_then(|id| self.clip(id))
        else {
            return SelectedClipData::default();
        };
        let text = &clip.text;
        SelectedClipData {
            present: true,
            id: clip.id.as_str().into(),
            name: clip.name.as_str().into(),
            kind: clip.kind,
            duration: clip.duration,
            scale: clip.scale,
            offset_x: clip.offset_x,
            offset_y: clip.offset_y,
            rotation: clip.rotation,
            opacity: clip.opacity,
            volume: clip.volume,
            speed: clip.speed,
            preserve_pitch: clip.preserve_pitch,
            fade_in: clip.fade_in,
            fade_out: clip.fade_out,
            content: clip.text_body.as_str().into(),
            font_family: text.family.as_str().into(),
            font_size: text.size,
            font_weight: text.weight,
            italic: text.italic,
            fill: text.fill,
            fill_hex: hex_of(text.fill).into(),
            align: text.align,
            text_opacity: text.opacity,
            stroke_width: text.stroke_width,
            stroke: text.stroke,
            stroke_hex: hex_of(text.stroke).into(),
            shadow: text.shadow,
            plate: text.plate,
            plate_hex: hex_of(text.plate).into(),
            plated: text.plate.alpha() > 0,
            line_height: text.line_height,
            tracking: text.tracking,
        }
    }

    /// The right-click menu for the clip it was opened on.
    ///
    /// Rows are built here rather than in Slint for the reason the bin's
    /// filtering is: what belongs on the menu depends on the clip, and
    /// deciding that needs branching over the model. `speed`'s three options
    /// are a flat labelled group rather than a submenu — a Slint struct cannot
    /// hold an array, so a row cannot carry children.
    fn menu(&self) -> Vec<MenuItemData> {
        let Some(clip) = self.menu_target.as_ref().and_then(|id| self.clip(id)) else {
            return Vec::new();
        };
        let locked = self.locked(&clip.track);
        let straddled = clip.start < self.playhead && self.playhead < clip.end();

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
            // Disabled rather than hidden: a menu whose rows move around
            // between openings has to be re-read every time.
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
        // No speed group. Three fixed rates is a worse control than the Adjust
        // tab's rate slider, which covers the engine's whole range, and a
        // right-click menu earns its length by holding the things that have
        // nowhere better to live.
        // Mute is the gain, not a flag: there is one number that decides
        // whether a clip is heard, and a second one would have to agree with it.
        // A still has no sound and a filter layer has no sound of its own —
        // it treats what is under it — so neither offers a gain to cut.
        let audible = !matches!(clip.kind, ClipKind::Image | ClipKind::Filter);
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

    /// What those rows add up to. Slint cannot sum a model, and the surface
    /// has to be given a height before it is shown.
    fn menu_height(rows: &[MenuItemData]) -> f32 {
        let metrics = |kind: MenuRow| match kind {
            MenuRow::Action => 26.0,
            MenuRow::Label => 24.0,
            MenuRow::Separator => 9.0,
        };
        rows.iter().map(|row| metrics(row.kind)).sum::<f32>() + 8.0
    }

    /// What one export tier would weigh, in bytes.
    fn export_size(&self, tier: usize) -> f32 {
        let (width, height) = EXPORT_SIZES[self.export.resolution.min(3)];
        let rate = EXPORT_RATES[self.export.rate.min(2)];
        let pixels = (width as f32 * height as f32) / (1920.0 * 1080.0);
        let video = EXPORT_TIERS[tier.min(2)] * 1_000_000.0 * pixels * (rate / 30.0);
        (video + AUDIO_BPS) * self.duration().max(1.0) / 8.0
    }

    fn export_data(&self) -> ExportData {
        let (width, height) = EXPORT_SIZES[self.export.resolution.min(3)];
        let rate = EXPORT_RATES[self.export.rate.min(2)];
        let clips = self.now().clips.len();
        let titles = self
            .now()
            .clips
            .iter()
            .filter(|clip| clip.kind == ClipKind::Text)
            .count();
        // The stage names follow the progress rather than being counted off:
        // the engine reports one number, and what it is busy with at that
        // number is a fact about the pipeline, not about the UI.
        let stage = if self.export.progress < 0.68 {
            "Rendering video"
        } else if self.export.progress < 0.88 {
            "Encoding audio"
        } else {
            "Finalising file"
        };

        ExportData {
            open: self.export.open,
            name: self.export.name.as_str().into(),
            path: format!("{}/{}.mp4", self.export.folder, self.export.name).into(),
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
            size_high: bytes(self.export_size(0)).into(),
            size_balanced: bytes(self.export_size(1)).into(),
            size_small: bytes(self.export_size(2)).into(),
            phase: self.export.phase,
            progress: self.export.progress,
            stage: stage.into(),
            // The rate is the simulation's: a real one comes off the engine.
            eta: eta((1.0 - self.export.progress) * 9.0).into(),
            message: self.export.message.as_str().into(),
            done_size: bytes(self.export_size(self.export.quality)).into(),
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
                    progress: if total > 0.0 { fetched / total } else { 0.0 },
                    transferred: format!("{fetched:.0} MB of {total:.0} MB · 12 MB/s").into(),
                    eta: eta((total - fetched) / 12.0).into(),
                }
            })
            .collect()
    }

    /// The tray's A/V menu. What it offers depends on what is selected, which
    /// is why it is built here and not declared in the tray.
    fn av_menu(&self) -> Vec<MenuItemData> {
        let Some(clip) = self
            .selection
            .first()
            .filter(|_| self.selection.len() == 1)
            .and_then(|id| self.clip(id))
        else {
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

        // A title's one tool is to speak its words; it has no sound to detach.
        if clip.kind == ClipKind::Text {
            return vec![row("speak", "Generate voice", Glyph::Volume, true)];
        }
        // A filter layer has neither: no words to speak and no track to lift
        // out of it. The tray hides the button rather than opening an empty
        // menu — see has_av_tools.
        if clip.kind == ClipKind::Filter {
            return Vec::new();
        }

        let has_sound = clip.kind != ClipKind::Image;
        let detached = self
            .now()
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
                clip.kind == ClipKind::Video && !detached,
            ),
            row(
                "reattach",
                "Reattach audio",
                Glyph::Merge,
                (clip.kind == ClipKind::Video && detached)
                    || (clip.kind == ClipKind::Audio && clip.detached_from.is_some()),
            ),
        ]
    }

    /// The File / Edit / View menus, following the reference's own lists.
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
        let straddled = self.now().clips.iter().any(|clip| {
            clip.start + MIN_DURATION < self.playhead && self.playhead < clip.end() - MIN_DURATION
        });

        match self.open_menu {
            0 => vec![
                // Disabled rather than absent: a menu whose rows move between
                // openings has to be re-read every time.
                row(
                    "add-selected",
                    "Add selected to timeline".into(),
                    Glyph::Plus,
                    "",
                    false,
                ),
                row("save", "Save".into(), Glyph::Import, "⌘S", true),
                row(
                    "export",
                    "Export…".into(),
                    Glyph::Export,
                    "",
                    !self.now().clips.is_empty(),
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
                // Nothing in this tree records what it did, so neither of
                // these can ever be live. They are here because a menu that
                // omits Undo is a menu that looks broken.
                row("undo", "Undo".into(), Glyph::ChevronUp, "⌘Z", false),
                row("redo", "Redo".into(), Glyph::ChevronDown, "⇧⌘Z", false),
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

    /// Everything the window shows.
    /// Raises the bottom-right notice. Every caller is some other handler
    /// that has just done — or refused to do — the thing it is reporting, so
    /// this only records; the publish that handler was going to make anyway is
    /// what puts it on screen.
    fn notify(&mut self, message: &str, failed: bool) {
        self.toast.token += 1;
        self.toast.message = message.into();
        self.toast.failed = failed;
    }

    fn publish(&self, app: &App, models: &Models) {
        self.publish_lanes(app, models);
        self.publish_chrome(app, models);
        self.publish_dock(app, models);
    }

    /// The workspace's arrangement, walked into boxes.
    ///
    /// Its own publisher, like the lanes, and for the same reason in reverse:
    /// dragging a gutter runs at pointer rate and changes nothing but this,
    /// while everything else in the window — the clips, the menus, the
    /// dialogs — changes without moving a single seat.
    fn publish_dock(&self, _app: &App, models: &Models) {
        let out = self.dock_layout();
        sync(&models.seats, out.seats);
        sync(&models.dividers, out.dividers);
    }

    /// The tree walked into the box Slint last reported.
    ///
    /// Walked afresh wherever it is wanted rather than cached: it is a handful
    /// of nodes, and a cache would be one more thing that can disagree with
    /// the tree — the same call the lane heights make.
    fn dock_layout(&self) -> DockLayout {
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

    /// The usable extent of one split — what a divider drag's pixels are a
    /// fraction of.
    fn split_extent(&self, index: usize) -> Option<f32> {
        self.dock_layout().extents.get(index).copied()
    }

    /// The ratio a split is currently at.
    fn split_ratio(&self, index: usize) -> Option<f32> {
        match self.dock.at(&self.dock.split_path(index)?) {
            Dock::Split { ratio, .. } => Some(*ratio),
            _ => None,
        }
    }

    /// The timeline and the readouts that follow it.
    ///
    /// Split from the chrome because this is what runs on every event of a
    /// scrub, a drag, a trim or a knob: sixty times a second while a pointer
    /// is down. Nothing a menu, a dialog or the engine lists show can have
    /// changed under a pointer that is dragging a clip, and rebuilding them
    /// anyway — three menus, the export panel's half-dozen formatted strings,
    /// both engine lists — is what made the playhead trail the pointer and
    /// then catch up in a jump.
    fn publish_lanes(&self, app: &App, models: &Models) {
        let editor = app.global::<Editor>();
        sync(
            &models.tabs,
            self.timelines
                .iter()
                .map(|timeline| TimelineTabData {
                    id: timeline.id.as_str().into(),
                    name: timeline.name.as_str().into(),
                })
                .collect(),
        );

        // Reversed on the way out: top-most lane first is what the panel
        // draws. Each row carries its own height and the running total above
        // it, because the stack is no longer a multiple of one pitch and a
        // prefix sum over a model is not something Slint can express.
        let mut top = 0.0;
        sync(
            &models.tracks,
            self.now()
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
                        locked: lane.locked,
                        size: lane.size,
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
            self.now()
                .clips
                .iter()
                .map(|clip| ClipData {
                    id: clip.id.as_str().into(),
                    name: clip.name.as_str().into(),
                    kind: clip.kind,
                    row: self.row_of(&clip.track),
                    start: clip.start,
                    duration: clip.duration,
                    selected: self.selection.iter().any(|id| id == &clip.id),
                    fx: clip.fx,
                    transition_in: clip.transition_in,
                    fade_in: clip.fade_in,
                    fade_out: clip.fade_out,
                    volume: clip.volume,
                    text_body: clip.text_body.as_str().into(),
                    wave: if clip.kind == ClipKind::Audio {
                        self.wave(clip)
                    } else {
                        SharedString::new()
                    },
                })
                .collect(),
        );

        // What is under the playhead. The topmost picture wins, the way the
        // compositor stacks them — a title over footage shows the title.
        let showing = self
            .now()
            .clips
            .iter()
            .filter(|clip| {
                matches!(
                    clip.kind,
                    ClipKind::Video | ClipKind::Image | ClipKind::Text
                ) && clip.start <= self.playhead
                    && self.playhead < clip.end()
            })
            .max_by_key(|clip| -self.row_of(&clip.track));
        editor.set_has_picture(showing.is_some());
        editor.set_preview_clip_name(
            showing
                .map(|clip| clip.name.as_str())
                .unwrap_or_default()
                .into(),
        );
        editor.set_preview_duration(self.duration());
        editor.set_playing(self.playing);

        // The ghost. Published as a whole rather than as an `active` flag
        // beside five stale numbers: the lanes draw it or they do not, and a
        // half-updated one would draw the last drag in the place of this one.
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
        editor.set_timeline_current_tab(self.active as i32);
        editor.set_playhead(self.playhead);
        editor.set_scroll_left(self.scroll_left);
        editor.set_seconds_per_pixel(self.seconds_per_pixel);
        editor.set_frame_rate(self.frame_rate);
        editor.set_tool(self.tool);
        editor.set_snap(self.snap);
        editor.set_selected_count(self.selection.len() as i32);
        editor.set_has_av_tools(self.has_av_tools());
        editor.set_merge_blocked_because(match self.merge_blocked() {
            Some(reason) => reason.into(),
            None => SharedString::new(),
        });
    }

    /// The menus, the dialogs and the engine lists.
    ///
    /// Every one of these is opened by a click, and every path that can open
    /// one goes through the full `publish` — so the rows are always rebuilt
    /// before the surface that shows them appears.
    fn publish_chrome(&self, app: &App, models: &Models) {
        let editor = app.global::<Editor>();

        app.set_on_start(self.on_start);
        app.set_project_name(self.project_name.as_str().into());
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
                        project.rate_num as f32 / project.rate_den as f32
                    )
                    .into(),
                    when: project.when.as_str().into(),
                    // No decoder here, so no poster: the row falls back to its
                    // film mark, which is what the reference does for a project
                    // with nothing visual on it or whose media has moved.
                    ..Default::default()
                })
                .collect(),
        );
        let (width, height) = OUTPUTS[self.ratio.min(OUTPUTS.len() - 1)];
        editor.set_output_width(width);
        editor.set_output_height(height);
        editor.set_ratio_index(self.ratio as i32);
        editor.set_quality_index(self.quality as i32);

        let rows = self.menu();
        editor.set_menu_height(Studio::menu_height(&rows));
        sync(&models.menu, rows);
        editor.set_menu_token(self.menu_token);

        app.set_export(self.export_data());
        app.set_settings(SettingsData {
            open: self.settings.open,
            tab: self.settings.tab,
            language: self.settings.language as i32,
            transcribe_language: self.settings.transcribe_language,
            disk: {
                let on_disk: f32 = self
                    .transcribers
                    .iter()
                    .chain(self.voices.iter())
                    .filter(|model| model.installed)
                    .map(|model| model.megabytes)
                    .sum();
                let count = self
                    .transcribers
                    .iter()
                    .chain(self.voices.iter())
                    .filter(|model| model.installed)
                    .count();
                format!("{count} installed · {on_disk:.0} MB on disk").into()
            },
            version: "0.1.0".into(),
            engine: "concat-engine · ffmpeg 7.1".into(),
        });
        sync(&models.transcribers, Studio::model_rows(&self.transcribers));
        sync(&models.voices, Studio::model_rows(&self.voices));

        let av = self.av_menu();
        editor.set_av_height(Studio::menu_height(&av));
        sync(&models.av, av);
        editor.set_av_token(self.av_token);

        let bar = self.menu_bar();
        app.set_app_menu_height(Studio::menu_height(&bar));
        sync(&models.bar, bar);
        app.set_app_menu_token(self.menu_bar_token);
        app.set_open_menu(self.open_menu);
    }
}

/// The audio envelope, as SVG path commands in a 1x1 box.
///
/// Columns, not a curve. The reference fills one rectangle per pixel of clip
/// width — `context.rect(x + column, top, 1, ...)` for every column, then one
/// fill — so its waveform is a stepped silhouette with a flat top on every
/// column. Drawing the same peaks as a polyline gives a smooth, rounded shape
/// that reads as a graph of something rather than as audio.
///
/// There is no decoder in this tree, so the peaks are synthesised from the
/// media id — deterministic, so a clip keeps its shape across a republish, and
/// two clips cut from one file agree where they were cut. Swap this for the
/// real buckets the day something decodes; the path it returns is the whole
/// contract.
///
/// Normalised rather than drawn in pixels so a zoom costs nothing: the Path
/// that renders it stretches the box onto the clip's current width.
fn wave_path(media: &str, source_start: f32, duration: f32, gain: f32) -> String {
    /// Columns across the clip. The reference gets one per pixel because it
    /// redraws at the clip's current width; this is drawn once and stretched,
    /// so the count is fixed — enough that the steps read as columns and not
    /// as a bar chart, few enough that the string stays a few kilobytes.
    const COLUMNS: usize = 128;
    /// Silence still draws a sliver, the way the reference's `max(1, ...)`
    /// keeps a one-pixel line rather than a gap in the middle of a clip.
    const FLOOR: f32 = 0.012;

    if duration <= 0.0 {
        return String::new();
    }

    // xorshift32 over a seed folded from the id, so no rand dependency and no
    // shared state between clips.
    let mut seed = media.bytes().fold(0x2545_f491_u32, |hash, byte| {
        (hash ^ byte as u32).wrapping_mul(16_777_619)
    }) | 1;
    let mut random = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed >> 8) as f32 / (1 << 24) as f32
    };

    // Drawn amplitude follows the clip's gain, so turning a clip up makes its
    // waveform visibly taller. Clamped, or a boosted clip would bleed into the
    // lane above.
    let gain = gain.max(0.0);
    let mut path = String::with_capacity(COLUMNS * 56);

    for column in 0..COLUMNS {
        let left = column as f32 / COLUMNS as f32;
        let right = (column + 1) as f32 / COLUMNS as f32;
        // A slow syllabic swell under the noise, so it reads as speech rather
        // than as static. Phase follows the in-point, which is what makes two
        // clips cut from one file line up where they were cut.
        let at = source_start + (column as f32 / COLUMNS as f32) * duration;
        let envelope = 0.35 + 0.3 * (at * 2.3).sin().abs() + 0.25 * random();
        let amplitude = ((envelope * gain).clamp(0.0, 1.0) * 0.48).max(FLOOR);
        let (top, bottom) = (0.5 - amplitude, 0.5 + amplitude);

        // Columns butt up against each other rather than leaving a gap: the
        // reference's are contiguous, which is what makes a loud passage read
        // as a solid block instead of a comb.
        path.push_str(&format!(
            "M {left:.4} {top:.4} L {right:.4} {top:.4} \
             L {right:.4} {bottom:.4} L {left:.4} {bottom:.4} Z "
        ));
    }

    path
}

/// The marks a drag chip can wear — lucide's own path data, the same source as
/// ui/icons.slint. Duplicated rather than reached for because the glyphs live
/// in the Slint tree as `Path` elements, and an element cannot be read back
/// out as a string.
fn chip_glyph(kind: ClipKind) -> &'static str {
    match kind {
        // lucide/film
        ClipKind::Video => {
            "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2Z \
                            M7 3v18 M3 7.5h4 M3 12h18 M3 16.5h4 M17 3v18 M17 7.5h4 M17 16.5h4"
        }
        // lucide/music
        ClipKind::Audio => {
            "M9 18V5l12-2v13 M3 18a3 3 0 1 0 6 0a3 3 0 1 0 -6 0 \
                            M15 16a3 3 0 1 0 6 0a3 3 0 1 0 -6 0"
        }
        // lucide/image
        ClipKind::Image => {
            "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2Z \
                            M7 9a2 2 0 1 0 4 0a2 2 0 1 0 -4 0 M21 15l-3.086-3.086a2 2 0 0 0-2.828 0L6 21"
        }
        // lucide/type
        ClipKind::Text => "M12 4v16 M4 7V5a1 1 0 0 1 1-1h14a1 1 0 0 1 1 1v2 M9 20h6",
        // lucide/audio-lines
        ClipKind::Filter => "M2 10v3 M6 6v11 M10 3v18 M14 8v7 M18 5v13 M22 10v3",
    }
}

/// The same again for a panel being dragged out of its seat. The slugs are
/// the ones `Panes.id` hands out, and they are what the payload carries.
fn pane_glyph(slug: &str) -> &'static str {
    match slug {
        // lucide/image
        "preview" => {
            "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2Z \
                      M7 9a2 2 0 1 0 4 0a2 2 0 1 0 -4 0 M21 15l-3.086-3.086a2 2 0 0 0-2.828 0L6 21"
        }
        // lucide/sliders-horizontal
        "inspector" => {
            "M10 5H3 M12 19H3 M14 3v4 M16 17v4 M21 12h-9 M21 19h-5 M21 5h-7 \
                        M8 10v4 M8 12H3"
        }
        // lucide/rows-3
        "timeline" => {
            "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2Z \
                       M21 9H3 M21 15H3"
        }
        // lucide/film
        _ => {
            "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2Z \
              M7 3v18 M3 7.5h4 M3 12h18 M3 16.5h4 M17 3v18 M17 7.5h4 M17 16.5h4"
        }
    }
}

/// `&`, `<` and `>` out of a name that is going into an SVG document.
fn xml_escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The chip that hangs off the cursor while something is dragged out of the
/// library, as an SVG document.
///
/// SVG rather than a pixel buffer, and for two reasons. Slint rasterises one
/// through the window's own font collection — so the name on the chip is set
/// in the same Inter the cards are, from the face this binary already
/// embeds — and the drag overlay draws it at the window's scale factor rather
/// than at whatever resolution a buffer was baked at.
///
/// Sized in logical points, because that is how the overlay reads it back:
/// `render_drag_image_overlay` takes the image's own size as its size on
/// screen. `PAD` is the margin the drop shadow needs to fall into; the visible
/// chip is inset by it, which is why the offsets the DragAreas pass are that
/// much larger than the gap they want.
fn drag_chip_svg(
    glyph: &str,
    label: &str,
    wave: &str,
    mark: slint::Color,
    well: slint::Color,
    ground: slint::Color,
    ink: slint::Color,
) -> String {
    /// Room for the shadow to fall into, on every side.
    const PAD: f32 = 5.0;
    const CHIP_H: f32 = 32.0;
    /// The badge, and the glyph centred in it.
    const BADGE: f32 = 22.0;
    const MARK: f32 = 16.0;
    /// Inter's average advance at 12px, rounded up. A chip a few points wider
    /// than its text is a chip; one a few points narrower is a bug.
    const ADVANCE: f32 = 6.4;
    /// Long enough for a take name, short enough not to become a banner.
    const MAX_CHARS: usize = 26;

    let label: String = if label.chars().count() > MAX_CHARS {
        label.chars().take(MAX_CHARS - 1).collect::<String>() + "\u{2026}"
    } else {
        label.to_string()
    };
    let text_x = PAD + 10.0 + BADGE;
    let chip_w = (text_x - PAD + label.chars().count() as f32 * ADVANCE + 12.0).clamp(96.0, 268.0);
    let (width, height) = (chip_w + 2.0 * PAD, CHIP_H + 2.0 * PAD);

    // The badge holds the file's own envelope when there is one, and the
    // kind's mark otherwise — the same rule the bin card follows, so what is
    // in the air looks like the card it came off.
    let badge_art = if wave.is_empty() {
        format!(
            r#"<g transform="translate({x} {y}) scale({scale})" fill="none" stroke="{mark}"
stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="{glyph}"/></g>"#,
            x = PAD + 5.0 + (BADGE - MARK) / 2.0,
            y = PAD + 5.0 + (BADGE - MARK) / 2.0,
            scale = MARK / 24.0,
            mark = hex_of(mark),
        )
    } else {
        format!(
            r#"<g transform="translate({x} {y}) scale({BADGE})"><path d="{wave}" fill="{mark}"/></g>"#,
            x = PAD + 5.0,
            y = PAD + 5.0,
            mark = hex_of(mark),
        )
    };

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">
<filter id="drop" x="-30%" y="-30%" width="170%" height="170%">
<feDropShadow dx="0" dy="1.5" stdDeviation="2" flood-color="#000000" flood-opacity="0.55"/></filter>
<rect x="{cx}" y="{cy}" width="{cw}" height="{ch}" rx="8" fill="{ground}" stroke="{mark}" stroke-opacity="0.65" filter="url(#drop)"/>
<rect x="{bx}" y="{by}" width="{BADGE}" height="{BADGE}" rx="6" fill="{well}"/>
{badge_art}
<text x="{tx}" y="{ty}" font-family="Inter" font-size="12" fill="{ink}">{label}</text>
</svg>"##,
        cx = PAD + 0.5,
        cy = PAD + 0.5,
        cw = chip_w - 1.0,
        ch = CHIP_H - 1.0,
        bx = PAD + 5.0,
        by = PAD + 5.0,
        tx = text_x,
        // The baseline, not the middle: usvg honours `dominant-baseline`
        // unevenly, and a number here is one fewer thing to be surprised by.
        ty = PAD + CHIP_H / 2.0 + 4.2,
        ground = hex_of(ground),
        mark = hex_of(mark),
        well = hex_of(well),
        ink = hex_of(ink),
        label = xml_escaped(&label),
    )
}

// ── the launch screen ────────────────────────────────────────────────────────

/// The frame sizes the launch screen offers, and what each label means.
/// The labels are the only half Slint sees; everything that has to be a number
/// stays here, so there is one place that knows 4K is 3840 x 2160.
const RESOLUTIONS: [(&str, i32, i32); 4] = [
    ("1080p", 1920, 1080),
    ("720p", 1280, 720),
    ("4K", 3840, 2160),
    ("Vertical", 1080, 1920),
];

/// The frame rates, as exact fractions. 29.97 is 30000/1001 and never anything
/// else — a decimal is for reading, and export must never be handed one.
const START_RATES: [(&str, i32, i32); 5] = [
    ("24", 24, 1),
    ("25", 25, 1),
    ("29.97", 30000, 1001),
    ("30", 30, 1),
    ("60", 60, 1),
];

/// The form on the launch screen.
struct StartState {
    name: String,
    location: String,
    resolution: usize,
    rate: usize,
    busy: bool,
    error: String,
}

impl Default for StartState {
    fn default() -> Self {
        Self {
            name: "Untitled project".into(),
            // Desktop/Concat, the way the reference resolves it. Failing to
            // find a home directory is not worth surfacing: the field starts
            // empty, Create stays refused until something is typed, and
            // Choose would fill it in on a build that has a picker.
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

/// A project that has been open before.
///
/// No store behind it in this tree, so `when` is a string rather than a
/// timestamp: with nothing writing the list there is nothing for a clock to
/// measure against, and the row wants a coarse phrase either way.
struct RecentProject {
    path: String,
    name: String,
    width: i32,
    height: i32,
    rate_num: i32,
    rate_den: i32,
    when: String,
}

fn recent(path: &str, name: &str, width: i32, height: i32, when: &str) -> RecentProject {
    RecentProject {
        path: path.into(),
        name: name.into(),
        width,
        height,
        rate_num: 30,
        rate_den: 1,
        when: when.into(),
    }
}

fn demo_recents() -> Vec<RecentProject> {
    vec![
        recent(
            "~/Desktop/Concat/untitled-project",
            "Untitled project",
            1080,
            1920,
            "yesterday",
        ),
        recent(
            "~/Desktop/Concat/untitled-project-2",
            "Untitled project",
            1080,
            1920,
            "4 days ago",
        ),
        recent(
            "~/Desktop/Concat/untitled-project-3",
            "Untitled project",
            1920,
            1080,
            "4 days ago",
        ),
        recent(
            "~/Desktop/Concat/untitled-project-4",
            "Untitled project",
            1920,
            1080,
            "5 days ago",
        ),
    ]
}

// ── the workspace layout ─────────────────────────────────────────────────────
//
// The arrangement of views is a tree: a node is either one view or a split of
// two nodes, side by side or one over the other. Dropping a panel on the edge
// of a seat turns that seat into a split with the panel on the named side,
// which is how a layout gets a shape the window did not start with — and it
// can happen again inside either half, to any depth.
//
// It lives here rather than in Slint because Slint has no recursion: a
// component cannot contain itself, so a nested layout cannot be drawn by
// descending one. What crosses the boundary is this tree already walked flat —
// see `lay_out`, and SeatBox/DockDivider in ui/dock.slint.

/// Smallest a seat may be squeezed to by a divider drag. The old fixed layout
/// carried the same two numbers as `min-col` and `min-row`.
const SEAT_MIN_W: f32 = 240.0;
const SEAT_MIN_H: f32 = 140.0;
/// The floor that holds where the one above cannot: enough of a seat left to
/// find its corner button with, which is the only way back from a layout that
/// has been squeezed too far.
const SEAT_MIN_GRAB: f32 = 64.0;
/// The gutter between the halves of a split, and the margin round the lot.
const SEAT_GAP: f32 = 8.0;

enum Dock {
    /// One view, filling its box.
    Leaf(PaneKind),
    /// Two nodes and the gutter between them. `columns` means side by side,
    /// which is a vertical gutter dragged left and right; `ratio` is the share
    /// of the usable extent — the box less the gutter — that the first takes.
    Split {
        columns: bool,
        ratio: f32,
        first: Box<Dock>,
        second: Box<Dock>,
    },
}

impl Dock {
    fn leaf(kind: PaneKind) -> Box<Dock> {
        Box::new(Dock::Leaf(kind))
    }

    /// The node a path names, following each step into the first branch or the
    /// second. A path that runs past a leaf stops there, which cannot happen
    /// for a path this module produced.
    fn at(&self, path: &[bool]) -> &Dock {
        let Some((step, rest)) = path.split_first() else {
            return self;
        };
        match self {
            Dock::Split { first, second, .. } => {
                if *step {
                    second.at(rest)
                } else {
                    first.at(rest)
                }
            }
            leaf => leaf,
        }
    }

    fn at_mut(&mut self, path: &[bool]) -> &mut Dock {
        let Some((step, rest)) = path.split_first() else {
            return self;
        };
        match self {
            Dock::Split { first, second, .. } => {
                if *step {
                    second.at_mut(rest)
                } else {
                    first.at_mut(rest)
                }
            }
            leaf => leaf,
        }
    }

    /// The path to the nth leaf, counted in the same walk `lay_out` uses — so
    /// the number a seat carries and the number a drop reports are the same
    /// number. Paths rather than indices for the editing below, because an
    /// index is invalidated by a change elsewhere in the tree and a path is
    /// only invalidated by a change on its own way down.
    fn leaf_path(&self, index: usize) -> Option<Vec<bool>> {
        fn walk(node: &Dock, want: usize, seen: &mut usize, path: &mut Vec<bool>) -> bool {
            match node {
                Dock::Leaf(_) => {
                    if *seen == want {
                        return true;
                    }
                    *seen += 1;
                    false
                }
                Dock::Split { first, second, .. } => {
                    path.push(false);
                    if walk(first, want, seen, path) {
                        return true;
                    }
                    path.pop();
                    path.push(true);
                    if walk(second, want, seen, path) {
                        return true;
                    }
                    path.pop();
                    false
                }
            }
        }
        let (mut seen, mut path) = (0, Vec::new());
        walk(self, index, &mut seen, &mut path).then_some(path)
    }

    /// The same for the nth split — parent before children, which is the order
    /// `lay_out` numbers the gutters in.
    fn split_path(&self, index: usize) -> Option<Vec<bool>> {
        fn walk(node: &Dock, want: usize, seen: &mut usize, path: &mut Vec<bool>) -> bool {
            let Dock::Split { first, second, .. } = node else {
                return false;
            };
            if *seen == want {
                return true;
            }
            *seen += 1;
            path.push(false);
            if walk(first, want, seen, path) {
                return true;
            }
            path.pop();
            path.push(true);
            if walk(second, want, seen, path) {
                return true;
            }
            path.pop();
            false
        }
        let (mut seen, mut path) = (0, Vec::new());
        walk(self, index, &mut seen, &mut path).then_some(path)
    }

    fn kind_at(&self, index: usize) -> Option<PaneKind> {
        match self.at(&self.leaf_path(index)?) {
            Dock::Leaf(kind) => Some(*kind),
            _ => None,
        }
    }

    /// Turn one leaf into a split, with a new view on the named side of it.
    /// Half and half: the seat being split is the only thing that knows how
    /// much room there is, and half of it is the answer that needs no rule.
    fn split_leaf(&mut self, path: &[bool], kind: PaneKind, side: DockSide) {
        let node = self.at_mut(path);
        let existing = Box::new(std::mem::replace(node, Dock::Leaf(kind)));
        let (columns, first, second) = match side {
            DockSide::Left => (true, Dock::leaf(kind), existing),
            DockSide::Right => (true, existing, Dock::leaf(kind)),
            DockSide::Top => (false, Dock::leaf(kind), existing),
            _ => (false, existing, Dock::leaf(kind)),
        };
        *node = Dock::Split {
            columns,
            ratio: 0.5,
            first,
            second,
        };
    }

    /// Take a leaf out, and let its sibling take the space — the split it was
    /// half of stops existing. A path with nothing above it is the whole tree,
    /// which cannot be removed: a window with no seats has nowhere to put
    /// anything back.
    fn remove_leaf(&mut self, path: &[bool]) {
        let Some((last, above)) = path.split_last() else {
            return;
        };
        let parent = self.at_mut(above);
        let survivor = match parent {
            Dock::Split { first, second, .. } => {
                let keep = if *last { first } else { second };
                std::mem::replace(keep.as_mut(), Dock::Leaf(PaneKind::Media))
            }
            _ => return,
        };
        *parent = survivor;
    }
}

/// The tree walked flat: a box per view, a box per gutter.
#[derive(Default)]
struct DockLayout {
    seats: Vec<SeatBox>,
    dividers: Vec<DockDivider>,
    /// The usable extent of each split — its box less the gutter — in the same
    /// order as `dividers`. A divider drag is a delta in pixels and a ratio is
    /// a fraction, and this is what converts between them.
    extents: Vec<f32>,
}

fn lay_out(node: &Dock, (x, y, w, h): (f32, f32, f32, f32), out: &mut DockLayout) {
    match node {
        Dock::Leaf(kind) => out.seats.push(SeatBox {
            index: out.seats.len() as i32,
            kind: *kind,
            x,
            y,
            width: w.max(0.0),
            height: h.max(0.0),
        }),
        Dock::Split {
            columns,
            ratio,
            first,
            second,
        } => {
            // The slot is taken before the children are walked, so a gutter's
            // number is its split's position in a parent-first walk — which is
            // what `split_path` counts and what a drag reports back.
            let index = out.dividers.len();
            out.dividers.push(DockDivider::default());
            out.extents.push(0.0);

            let along = if *columns { w } else { h };
            let usable = (along - SEAT_GAP).max(0.0);
            let head = (usable * ratio).clamp(0.0, usable);
            out.extents[index] = usable;
            out.dividers[index] = if *columns {
                DockDivider {
                    index: index as i32,
                    columns: true,
                    x: x + head,
                    y,
                    width: SEAT_GAP,
                    height: h.max(0.0),
                }
            } else {
                DockDivider {
                    index: index as i32,
                    columns: false,
                    x,
                    y: y + head,
                    width: w.max(0.0),
                    height: SEAT_GAP,
                }
            };

            if *columns {
                lay_out(first, (x, y, head, h), out);
                lay_out(second, (x + head + SEAT_GAP, y, usable - head, h), out);
            } else {
                lay_out(first, (x, y, w, head), out);
                lay_out(second, (x, y + head + SEAT_GAP, w, usable - head), out);
            }
        }
    }
}

/// The arrangement the editor opens with: the library, the monitor and the
/// inspector across the top, the timeline along the bottom. The same three
/// shares the fixed layout carried, re-expressed as a tree — 0.31 of the width
/// to the library and 0.22 to the inspector, which is 0.319 of what is left
/// once the library has had its share.
fn demo_dock() -> Dock {
    Dock::Split {
        columns: false,
        ratio: 0.6,
        first: Box::new(Dock::Split {
            columns: true,
            ratio: 0.31,
            first: Dock::leaf(PaneKind::Media),
            second: Box::new(Dock::Split {
                columns: true,
                ratio: 0.681,
                first: Dock::leaf(PaneKind::Preview),
                second: Dock::leaf(PaneKind::Inspector),
            }),
        }),
        second: Dock::leaf(PaneKind::Timeline),
    }
}

/// The top edge of a row, measured down the stack from the first lane.
fn row_top(heights: &[f32], row: i32) -> f32 {
    heights.iter().take(row.max(0) as usize).sum()
}

/// The row a point down the stack falls in, clamped to the stack.
fn row_at(heights: &[f32], y: f32) -> i32 {
    let mut top = 0.0;
    for (row, height) in heights.iter().enumerate() {
        top += height;
        if y < top {
            return row as i32;
        }
    }
    (heights.len() as i32 - 1).max(0)
}

/// The row whose top edge is nearest a point down the stack.
///
/// What a *move* wants, and not the same question as `row_at`: dragging a clip
/// halfway into the lane below should put it there, which is the rounding the
/// old `delta / lane-height` did for free back when every lane was one height.
fn nearest_row(heights: &[f32], y: f32) -> i32 {
    let (mut top, mut best, mut best_distance) = (0.0_f32, 0_i32, f32::MAX);
    for (row, height) in heights.iter().enumerate() {
        let distance = (top - y).abs();
        if distance < best_distance {
            best_distance = distance;
            best = row as i32;
        }
        top += height;
    }
    best
}

/// A lane, at the defaults everything but the demo uses: seen, heard,
/// unlocked, and as tall as whatever ends up on it.
fn track(id: &str, name: &str) -> TrackDoc {
    TrackDoc {
        id: id.into(),
        name: name.into(),
        visible: true,
        muted: false,
        locked: false,
        size: TrackSize::Auto,
    }
}

fn clip(
    id: &str,
    name: &str,
    kind: ClipKind,
    track: &str,
    media: &str,
    start: f32,
    duration: f32,
) -> ClipDoc {
    ClipDoc {
        id: id.into(),
        name: name.into(),
        kind,
        track: track.into(),
        media: media.into(),
        preset: String::new(),
        start,
        duration,
        source_start: 0.0,
        scale: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
        rotation: 0.0,
        opacity: 1.0,
        speed: 1.0,
        preserve_pitch: true,
        volume: 1.0,
        fade_in: 0.0,
        fade_out: 0.0,
        fx: false,
        transition_in: false,
        text_body: String::new(),
        detached_from: None,
        text: TextStyleDoc::default(),
    }
}

/// A cut the size of a real one, so the lanes are exercised at the zooms and
/// widths a splitter drag actually produces.
fn demo_studio() -> Studio {
    let mut voice = clip("c1", "intro.mp3", ClipKind::Audio, "T1", "m1", 0.0, 3.4);
    voice.fade_in = 0.4;
    voice.fade_out = 0.6;
    let mut line = clip("c2", "line-02.mp3", ClipKind::Audio, "T1", "m13", 4.2, 1.9);
    line.fade_out = 0.3;
    let mut out = clip("c3", "outro.mp3", ClipKind::Audio, "T1", "m17", 7.0, 5.2);
    out.fade_in = 0.5;

    let mut wide = clip("c4", "0001-0036.mp4", ClipKind::Video, "T2", "m2", 0.0, 1.2);
    wide.fx = true;
    let mut mid = clip("c5", "0036-0072.mp4", ClipKind::Video, "T2", "m3", 1.2, 1.2);
    mid.transition_in = true;
    let long = clip("c6", "0072-0134.mp4", ClipKind::Video, "T2", "m4", 2.4, 2.1);
    let mut close = clip("c7", "0134-0182.mp4", ClipKind::Video, "T2", "m5", 5.1, 1.6);
    close.transition_in = true;
    close.fx = true;
    // Punched in a little, so the Transform group opens on something other
    // than four defaults the first time a video clip is selected.
    close.scale = 1.25;
    close.offset_y = -0.06;
    let tail = clip("c8", "0232-0322.mp4", ClipKind::Video, "T2", "m6", 8.2, 3.0);

    let mut title = clip("c9", "Title", ClipKind::Text, "T3", "", 0.6, 2.2);
    title.text_body = "Concat — a cut worth keeping".into();
    title.text = TextStyleDoc {
        family: "Synonym".into(),
        size: 0.11,
        weight: 600.0,
        ..Default::default()
    };
    let mut lower = clip("c10", "Lower third", ClipKind::Text, "T3", "", 5.4, 2.0);
    lower.text_body = "Chapter one".into();
    lower.text = TextStyleDoc {
        size: 0.06,
        weight: 500.0,
        align: TextAlignment::Left,
        fill: slint::Color::from_rgb_u8(0xcb, 0xf5, 0x3f),
        plate: slint::Color::from_argb_u8(0xcc, 0, 0, 0),
        ..Default::default()
    };
    // Lower thirds sit low in the frame, which is the whole of what makes them
    // lower thirds.
    lower.offset_y = 0.3;

    // A layer over the cut, so the stack opens with one lane of each height:
    // footage large, sound and titles medium, this minimal.
    let mut layer = clip("c12", "Telephone", ClipKind::Filter, "T4", "", 4.2, 3.4);
    layer.preset = "telephone".into();
    layer.fade_in = 0.3;
    layer.fade_out = 0.3;

    let timeline = TimelineDoc {
        id: "TL1".into(),
        name: "Timeline 1".into(),
        // Bottom-most first: T1 is the floor of the stack, T4 the top.
        tracks: vec![
            track("T1", "Track 1"),
            track("T2", "Track 2"),
            track("T3", "Track 3"),
            track("T4", "Track 4"),
        ],
        clips: vec![
            voice, line, out, wide, mid, long, close, tail, title, lower, layer,
        ],
    };

    Studio {
        waves: RefCell::new(std::collections::HashMap::new()),
        timelines: vec![
            timeline,
            TimelineDoc {
                id: "TL2".into(),
                name: "Rough cut".into(),
                tracks: vec![track("T5", "Track 1"), track("T6", "Track 2")],
                clips: vec![clip(
                    "c11",
                    "0326-0452.mp4",
                    ClipKind::Video,
                    "T6",
                    "m7",
                    0.0,
                    4.2,
                )],
            },
        ],
        active: 0,
        selection: Vec::new(),
        playhead: 2.0,
        scroll_left: 0.0,
        // ~20px a second: a ten-second cut fits a pane at its default width.
        seconds_per_pixel: 0.05,
        frame_rate: 30.0,
        tool: TimelineTool::Select,
        snap: true,
        ratio: 0,
        quality: 1,
        playing: false,
        clipboard: None,
        export: ExportState::default(),
        settings: SettingsState::default(),
        transcribers: demo_transcribers(),
        voices: demo_voices(),
        av_token: 0,
        open_menu: -1,
        menu_bar_token: 0,
        menu_target: None,
        menu_token: 0,
        on_start: true,
        start: StartState::default(),
        recents: demo_recents(),
        project_name: "Untitled project".into(),
        dock: demo_dock(),
        // Nothing until Slint says otherwise, which it does on its first
        // layout. Publishing an empty workspace until then is right: there is
        // no box to put a seat in yet.
        workspace: (0.0, 0.0),
        divider_press: None,
        gesture: Gesture::None,
        drop: None,
        toast: ToastState::default(),
        next_id: 100,
    }
}

/// The operating system, named the way an issue report needs it.
///
/// Asked of the system rather than inferred: `target_os` says "macos", which
/// is the one fact about the machine nobody ever has to be told. What is
/// wanted is the release, because that is what a rendering bug or a file
/// dialog behaving oddly turns out to depend on.
///
/// Spawning a process to find it out is fine here and only here: it happens
/// once, at startup, and the answer is kept for the life of the window.
fn os_description() -> String {
    #[cfg(target_os = "macos")]
    {
        // Bare `sw_vers` prints all three lines at once, so this is one
        // process rather than the three the flags would cost.
        let fields = std::process::Command::new("sw_vers")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default();
        let field = |key: &str| {
            fields
                .lines()
                .find_map(|line| line.strip_prefix(key)?.split(':').nth(1))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let name = field("ProductName").unwrap_or_else(|| "macOS".into());
        match (field("ProductVersion"), field("BuildVersion")) {
            (Some(version), Some(build)) => format!("{name} {version} ({build})"),
            (Some(version), None) => format!("{name} {version}"),
            _ => name,
        }
    }
    #[cfg(target_os = "linux")]
    {
        // The distribution's own name for itself, which is what a report
        // wants; the kernel version is the next question, not the first.
        if let Ok(release) = std::fs::read_to_string("/etc/os-release") {
            if let Some(pretty) = release
                .lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|value| value.trim_matches('"'))
                .filter(|value| !value.is_empty())
            {
                return pretty.to_owned();
            }
        }
        return "Linux".into();
    }
    #[cfg(target_os = "windows")]
    {
        return std::process::Command::new("cmd")
            .args(["/c", "ver"])
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "Windows".into());
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        std::env::consts::OS.to_owned()
    }
}

/// What Settings > About shows under "System information", and what its copy
/// button puts on the clipboard.
///
/// One list, read twice: the rows on screen and the block that is copied are
/// built from the same pairs, so a fact cannot be on the page and missing
/// from the report. Gathered once — every line of it is fixed for the life of
/// the process — and handed over as a model that is never replaced.
fn system_facts() -> Vec<(&'static str, String)> {
    vec![
        (
            "Application",
            format!("Concat {}", env!("CARGO_PKG_VERSION")),
        ),
        (
            "Build",
            format!("{} · {}", env!("BUILD_PROFILE"), env!("BUILD_TARGET")),
        ),
        // Which of the two renderers in Cargo.toml this binary was built
        // with. The first question to ask about anything that looks wrong on
        // screen, and the one nobody can answer by looking at the window.
        (
            "Renderer",
            if cfg!(feature = "skia") {
                "Skia"
            } else {
                "FemtoVG (wgpu)"
            }
            .into(),
        ),
        ("Engine", "concat-engine · ffmpeg 7.1".into()),
        ("Operating system", os_description()),
        (
            "Processor",
            format!(
                "{} · {} threads",
                std::env::consts::ARCH,
                std::thread::available_parallelism().map_or(0, |count| count.get())
            ),
        ),
        ("Toolchain", env!("BUILD_RUSTC").into()),
    ]
}

fn main() -> Result<(), slint::PlatformError> {
    // The custom title bar. On macOS the native bar is hidden and the traffic
    // lights are overlaid on the strip the UI draws — the winit spelling of
    // the Tauri build's `titleBarStyle: "Overlay"`. Other platforms keep
    // their decorations for now; the reference goes undecorated with its own
    // window buttons on Windows, which is a separate port.
    #[cfg(target_os = "macos")]
    {
        use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
        slint::BackendSelector::new()
            .backend_name("winit".into())
            // Metal, by name. Skia picks its surface from a cfg chain — vulkan,
            // opengl, metal, softbuffer — and on macOS that already lands on
            // metal_surface, so this changes nothing about which one is used.
            // What it changes is what happens when it cannot be had: the chain
            // walks on to Skia's CPU rasteriser and the app runs, slowly and
            // silently, at a fraction of the frame rate on a machine that has
            // a GPU. Requiring it turns that into a refusal to start, which is
            // a fault you can see. Same reason Windows requires D3D below,
            // though there it is also the only way to reach it.
            .require_metal()
            .with_winit_window_attributes_hook(|attributes| {
                attributes
                    .with_titlebar_transparent(true)
                    .with_title_hidden(true)
                    .with_fullsize_content_view(true)
            })
            .select()?;
    }

    // Windows renders on the CPU unless Direct3D is asked for by name. Skia
    // picks its surface from a cfg chain in i-slint-renderer-skia — vulkan ->
    // opengl -> metal -> softbuffer — that has no Direct3D arm at all, and
    // whose opengl arm is `not(any(target_vendor = "apple", target_family =
    // "windows", ...))`, i.e. explicitly not Windows. So with plain
    // renderer-skia every arm but the last fails here and DefaultSurface
    // resolves to software_surface: Skia's CPU rasteriser, even though
    // d3d_surface.rs is compiled in and SkiaRenderer::default_direct3d exists.
    // require_d3d() is the only thing that reaches it.
    //
    // macOS lands on metal_surface and Linux on opengl_surface from that same
    // chain, so both are already on the GPU and neither needs this.
    #[cfg(target_family = "windows")]
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .require_d3d()
        .select()?;

    let app = App::new()?;
    app.set_macos(cfg!(target_os = "macos"));

    // One handle for the whole of main. Everything the editor's views read and
    // report lives on this global rather than on the window — see
    // ui/editor.slint — and `global()` only borrows, so it can be held across
    // every binding below.
    let editor = app.global::<Editor>();

    // The strip's drag region and double-click. Only the winit window can do
    // either; the scene graph forwards the gestures here.
    app.on_titlebar_begin_drag({
        let weak = app.as_weak();
        move || {
            use slint::winit_030::WinitWindowAccessor;
            if let Some(app) = weak.upgrade() {
                app.window().with_winit_window(|window| {
                    let _ = window.drag_window();
                });
            }
        }
    });

    app.on_titlebar_toggle_maximize({
        let weak = app.as_weak();
        move || {
            use slint::winit_030::WinitWindowAccessor;
            if let Some(app) = weak.upgrade() {
                app.window().with_winit_window(|window| {
                    window.set_maximized(!window.is_maximized());
                });
            }
        }
    });

    // --- effect lists ----------------------------------------------------
    let video = Rc::new(VecModel::from(vec![
        EffectData {
            id: 1,
            name: SharedString::from("Lumetri Color"),
            audio: false,
        },
        EffectData {
            id: 2,
            name: SharedString::from("Motion Blur"),
            audio: false,
        },
        EffectData {
            id: 3,
            name: SharedString::from("Glow"),
            audio: false,
        },
    ]));
    let audio = Rc::new(VecModel::from(vec![
        EffectData {
            id: 4,
            name: SharedString::from("EQ"),
            audio: true,
        },
        EffectData {
            id: 5,
            name: SharedString::from("Denoiser"),
            audio: true,
        },
    ]));

    editor.set_video_effects(ModelRc::from(video.clone()));
    editor.set_audio_effects(ModelRc::from(audio.clone()));

    let next_id = Rc::new(Cell::new(6));

    editor.on_add_effect({
        let video = video.clone();
        let audio = audio.clone();
        let next_id = next_id.clone();
        move |is_audio| {
            let list = if is_audio { &audio } else { &video };
            let id = next_id.get();
            next_id.set(id + 1);
            let name = POOL[list.row_count() % POOL.len()];
            list.push(EffectData {
                id,
                name: SharedString::from(name),
                audio: is_audio,
            });
        }
    });

    editor.on_remove_effect({
        let video = video.clone();
        let audio = audio.clone();
        move |id| {
            for list in [&video, &audio] {
                if let Some(row) = list.iter().position(|effect| effect.id == id) {
                    list.remove(row);
                    return;
                }
            }
        }
    });

    // --- the bin ---------------------------------------------------------
    let library = Rc::new(Library::new(demo_library()));
    let view = Rc::new(VecModel::<MediaItemData>::default());
    editor.set_media(ModelRc::from(view.clone()));
    library.publish(&app, &view);

    // Every one of these ends in the same republish, so there is one place
    // where the panel's idea of the bin can go wrong.
    editor.on_media_filter_changed({
        let weak = app.as_weak();
        let library = library.clone();
        let view = view.clone();
        move |filter| {
            let Some(app) = weak.upgrade() else { return };
            library.filter.set(filter);
            library.publish(&app, &view);
        }
    });

    editor.on_media_select({
        let weak = app.as_weak();
        let library = library.clone();
        let view = view.clone();
        move |id, additive| {
            let Some(app) = weak.upgrade() else { return };
            for item in library.items.borrow_mut().iter_mut() {
                if item.id == id {
                    // A modifier toggles this one and leaves the rest alone; a
                    // plain click makes it the whole selection.
                    item.selected = !additive || !item.selected;
                } else if !additive {
                    item.selected = false;
                }
            }
            library.publish(&app, &view);
        }
    });

    // A marquee closed over the grid. The panel hands down the block of cells
    // it caught rather than a list of ids: the bin is placed by arithmetic on
    // that side, so that side is the only one that knows which cell a card is
    // in, and a rectangle of cells says the same thing in six numbers without
    // a model to carry it back.
    //
    // The walk is over the *filtered* order, because that is what the grid was
    // laid out from — the panel's third card is the third card the filter let
    // through, not the third row of the bin. Cards the filter is hiding are
    // not in the grid and so were never under the band: a plain sweep clears
    // them the way the reference's `onSelect([])` does, and a shift-sweep
    // leaves them exactly as they were.
    editor.on_media_band_selected({
        let weak = app.as_weak();
        let library = library.clone();
        let view = view.clone();
        move |columns, from_col, to_col, from_row, to_row, additive| {
            let Some(app) = weak.upgrade() else { return };
            let filter = library.filter.get();
            let mut cell = 0;
            for item in library.items.borrow_mut().iter_mut() {
                if !Library::shows(filter, item.kind) {
                    item.selected = additive && item.selected;
                    continue;
                }
                let (row, col) = (cell / columns.max(1), cell % columns.max(1));
                let caught = row >= from_row && row <= to_row && col >= from_col && col <= to_col;
                item.selected = caught || (additive && item.selected);
                cell += 1;
            }
            library.publish(&app, &view);
        }
    });

    editor.on_media_remove({
        let weak = app.as_weak();
        let library = library.clone();
        let view = view.clone();
        move |id| {
            let Some(app) = weak.upgrade() else { return };
            library.items.borrow_mut().retain(|item| item.id != id);
            library.publish(&app, &view);
        }
    });

    editor.on_media_remove_selected({
        let weak = app.as_weak();
        let library = library.clone();
        let view = view.clone();
        move || {
            let Some(app) = weak.upgrade() else { return };
            library.items.borrow_mut().retain(|item| !item.selected);
            library.publish(&app, &view);
        }
    });

    // No file picker in this crate — the reference opens Tauri's, and reaching
    // for a native dialog here is a dependency decision, not a detail of the
    // panel. The callback exists so the panel is wired to something the day
    // one lands. (`media-activate` is handled with the timeline's own drops,
    // further down: it needs the studio a double-click puts a clip into.)
    editor.on_import_media(|| {});

    // --- the timeline ----------------------------------------------------
    //
    // Every handler below ends in a republish, and there are two of them: the
    // lanes alone for anything a pointer can drag, and the whole window for
    // everything else. One path that always produces a consistent panel is
    // still worth more than row-level updates — but a gesture cannot afford
    // to rebuild the menus and the dialogs sixty times a second on its way
    // through.
    let studio = Rc::new(RefCell::new(demo_studio()));
    let models = Rc::new(Models {
        tabs: Rc::new(VecModel::default()),
        tracks: Rc::new(VecModel::default()),
        clips: Rc::new(VecModel::default()),
        menu: Rc::new(VecModel::default()),
        av: Rc::new(VecModel::default()),
        bar: Rc::new(VecModel::default()),
        transcribers: Rc::new(VecModel::default()),
        voices: Rc::new(VecModel::default()),
        seats: Rc::new(VecModel::default()),
        dividers: Rc::new(VecModel::default()),
        recents: Rc::new(VecModel::default()),
    });
    // Handed over once, here, and never replaced: a fresh model is a reset,
    // and a reset rebuilds every row that hangs off it.
    editor.set_timeline_tabs(ModelRc::from(models.tabs.clone()));
    editor.set_tracks(ModelRc::from(models.tracks.clone()));
    editor.set_clips(ModelRc::from(models.clips.clone()));
    editor.set_menu_items(ModelRc::from(models.menu.clone()));
    editor.set_av_items(ModelRc::from(models.av.clone()));
    app.set_app_menu_items(ModelRc::from(models.bar.clone()));
    app.set_transcribers(ModelRc::from(models.transcribers.clone()));
    app.set_voices(ModelRc::from(models.voices.clone()));
    editor.set_seats(ModelRc::from(models.seats.clone()));
    editor.set_dividers(ModelRc::from(models.dividers.clone()));
    app.set_recents(ModelRc::from(models.recents.clone()));

    // Settings > About's block, gathered once. Nothing in it can change while
    // the process runs, so it is set here beside the models that are handed
    // over rather than rebuilt on every publish — and the rows and the string
    // the copy button hands to the clipboard are built from the one list, so
    // a fact cannot be on the page and missing from the paste.
    let facts = system_facts();
    app.set_system_report(
        facts
            .iter()
            .map(|(label, value)| format!("{label}: {value}"))
            .collect::<Vec<_>>()
            .join("\n")
            .into(),
    );
    app.set_system_facts(ModelRc::from(Rc::new(VecModel::from(
        facts
            .into_iter()
            .map(|(label, value)| SystemFactData {
                label: label.into(),
                value: value.into(),
            })
            .collect::<Vec<_>>(),
    ))));

    // The two ladders' labels, handed over once: they are constants, and the
    // index the form reports back is what carries the meaning. See
    // RESOLUTIONS and START_RATES.
    app.set_start_resolutions(ModelRc::from(Rc::new(VecModel::from(
        RESOLUTIONS
            .iter()
            .map(|(label, _, _)| SharedString::from(*label))
            .collect::<Vec<_>>(),
    ))));
    app.set_start_rates(ModelRc::from(Rc::new(VecModel::from(
        START_RATES
            .iter()
            .map(|(label, _, _)| SharedString::from(*label))
            .collect::<Vec<_>>(),
    ))));

    // Mutate, then republish. Handed the weak handle rather than the app so a
    // callback outliving the window is a no-op instead of a panic.
    macro_rules! publishing {
        ($publish:ident, |$state:ident $(, $arg:ident : $ty:ty)*| $body:block) => {{
            let weak = app.as_weak();
            let studio = studio.clone();
            let models = models.clone();
            move |$($arg : $ty),*| {
                let Some(app) = weak.upgrade() else { return };
                {
                    let mut $state = studio.borrow_mut();
                    $body
                }
                studio.borrow().$publish(&app, &models);
            }
        }};
    }

    /// The whole window. What anything that opens a menu or a dialog uses.
    macro_rules! on_timeline {
        ($($handler:tt)*) => { publishing!(publish, $($handler)*) };
    }

    /// The lanes and the readouts that follow them, and nothing else. For the
    /// handlers a pointer drives directly — a scrub, a move, a trim, a knob —
    /// which arrive as a stream of events and must each cost as little as the
    /// change they carry.
    macro_rules! on_lanes {
        ($($handler:tt)*) => { publishing!(publish_lanes, $($handler)*) };
    }

    // ── the launch screen ──
    //
    // Everything here ends in the full publish: the form is chrome, and the
    // one thing it can do is stop being on screen.
    app.on_start_name_edited(on_timeline!(|state, name: SharedString| {
        state.start.name = name.to_string();
    }));
    app.on_start_location_edited(on_timeline!(|state, path: SharedString| {
        state.start.location = path.to_string();
    }));
    app.on_start_resolution_changed(on_timeline!(|state, index: i32| {
        state.start.resolution = (index.max(0) as usize).min(RESOLUTIONS.len() - 1);
    }));
    app.on_start_rate_changed(on_timeline!(|state, index: i32| {
        state.start.rate = (index.max(0) as usize).min(START_RATES.len() - 1);
    }));
    app.on_start_dismiss_error(on_timeline!(|state| {
        state.start.error.clear();
    }));

    // No folder picker in this crate, for the reason `import-media` has none:
    // reaching for a native dialog is a dependency decision and not a detail
    // of this screen. The seam is here so the field is wired to something the
    // day one lands.
    app.on_start_browse(|| {});

    // Creating a project is, here, choosing what the editor opens as. There is
    // no engine to write a manifest and no folder to make — so what the form
    // decides is what it can actually decide: the name in the title bar and
    // the rate and frame size the timeline and the monitor work in.
    app.on_start_create(on_timeline!(|state| {
        let name = state.start.name.trim().to_string();
        state.project_name = if name.is_empty() {
            "Untitled project".into()
        } else {
            name
        };

        let (_, width, height) = RESOLUTIONS[state.start.resolution];
        let (_, num, den) = START_RATES[state.start.rate];
        state.frame_rate = num as f32 / den as f32;
        // The monitor's list is a different ladder — a project's frame size is
        // not always one of the aspect presets — so this only moves it when
        // the two agree, and leaves it where it is when they do not.
        if let Some(index) = OUTPUTS.iter().position(|size| *size == (width, height)) {
            state.ratio = index;
        }
        state.on_start = false;
    }));

    // Reopening does what creating does, minus the form: the recent row
    // already carries the size and the rate the project was made with.
    app.on_start_open_recent(on_timeline!(|state, path: SharedString| {
        // Copied out before anything is written: the row lives in the same
        // struct as the fields being set from it.
        let Some((name, rate, size)) = state
            .recents
            .iter()
            .find(|entry| entry.path == path.as_str())
            .map(|entry| {
                (
                    entry.name.clone(),
                    entry.rate_num as f32 / entry.rate_den as f32,
                    (entry.width, entry.height),
                )
            })
        else {
            return;
        };
        state.project_name = name;
        state.frame_rate = rate;
        if let Some(index) = OUTPUTS.iter().position(|entry| *entry == size) {
            state.ratio = index;
        }
        state.on_start = false;
    }));

    app.on_start_forget_recent(on_timeline!(|state, path: SharedString| {
        state.recents.retain(|entry| entry.path != path.as_str());
    }));

    // ── the workspace's arrangement ──
    //
    // Its own publisher rather than the full one: nothing a seat does can
    // change a clip, a menu or a dialog, and a gutter drag runs at pointer
    // rate. See `publish_dock`.
    macro_rules! on_dock {
        ($($handler:tt)*) => { publishing!(publish_dock, $($handler)*) };
    }

    editor.on_workspace_resized(on_dock!(|state, width: f32, height: f32| {
        state.workspace = (width, height);
    }));

    // Picking from a seat's dropdown. An assignment and nothing else: the view
    // that was here is simply not here any more, and the same view may now be
    // in two seats at once. That is the difference between this and a drop —
    // see below, where there is somewhere for the incumbent to go.
    editor.on_dock_set(on_dock!(|state, seat: i32, kind: PaneKind| {
        let Some(path) = state.dock.leaf_path(seat.max(0) as usize) else {
            return;
        };
        if let Dock::Leaf(held) = state.dock.at_mut(&path) {
            *held = kind;
        }
    }));

    // A panel dropped on a seat. The middle trades the two views; an edge
    // splits the seat it landed on and takes that side of it.
    editor.on_dock_dropped(on_dock!(|state, from: i32, onto: i32, side: DockSide| {
        let (from, onto) = (from.max(0) as usize, onto.max(0) as usize);
        if from == onto {
            return;
        }
        let (Some(taken), Some(displaced)) = (state.dock.kind_at(from), state.dock.kind_at(onto))
        else {
            return;
        };

        if side == DockSide::Centre {
            for (index, kind) in [(from, displaced), (onto, taken)] {
                let Some(path) = state.dock.leaf_path(index) else {
                    continue;
                };
                if let Dock::Leaf(held) = state.dock.at_mut(&path) {
                    *held = kind;
                }
            }
            return;
        }

        // Both paths before either edit. Splitting `onto` only lengthens the
        // paths that run through `onto`, and `from` is a different leaf, so
        // its path survives the split — which is the whole reason these are
        // paths and not the indices they came in as.
        let (Some(onto_path), Some(from_path)) =
            (state.dock.leaf_path(onto), state.dock.leaf_path(from))
        else {
            return;
        };
        state.dock.split_leaf(&onto_path, taken, side);
        state.dock.remove_leaf(&from_path);
    }));

    // A view added from the window's own corner, which has no seat to be added
    // beside — so it halves whichever seat currently has the most room, along
    // that seat's longer side. That is what a tiling window manager does with
    // a new window, and it is the only rule that does not need the user to
    // have already decided something they were not asked.
    editor.on_dock_add(on_dock!(|state, kind: PaneKind| {
        let seats = state.dock_layout().seats;
        let Some(biggest) = seats
            .iter()
            .max_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)))
        else {
            return;
        };
        // The longer side, unless the longer side has no room to be halved and
        // the shorter one does — a tall narrow seat is cut across even when it
        // is the widest thing on screen, because the alternative is two seats
        // too narrow to carry a header.
        let across = biggest.width >= SEAT_MIN_W * 2.0;
        let down = biggest.height >= SEAT_MIN_H * 2.0;
        let side = if biggest.width >= biggest.height && (across || !down) {
            DockSide::Right
        } else {
            DockSide::Bottom
        };
        let Some(path) = state.dock.leaf_path(biggest.index.max(0) as usize) else {
            return;
        };
        state.dock.split_leaf(&path, kind, side);
    }));

    // A seat closed from its own dropdown. Its sibling takes the room, which
    // is the split the two were halves of ceasing to exist — `remove_leaf`
    // refuses the last one, so the window cannot be emptied.
    editor.on_dock_remove(on_dock!(|state, seat: i32| {
        let Some(path) = state.dock.leaf_path(seat.max(0) as usize) else {
            return;
        };
        state.dock.remove_leaf(&path);
    }));

    editor.on_divider_pressed(on_dock!(|state, index: i32| {
        let index = index.max(0) as usize;
        state.divider_press = match (state.split_ratio(index), state.split_extent(index)) {
            (Some(ratio), Some(extent)) => Some((index, ratio, extent)),
            _ => None,
        };
    }));

    editor.on_divider_dragged(on_dock!(|state, index: i32, delta: f32| {
        let Some((held, from, extent)) = state.divider_press else {
            return;
        };
        if held != index.max(0) as usize || extent <= 0.0 {
            return;
        }
        let Some(path) = state.dock.split_path(held) else {
            return;
        };
        let Dock::Split { columns, ratio, .. } = state.dock.at_mut(&path) else {
            return;
        };
        // The floor is in pixels and the ratio is a fraction, so the clamp is
        // the floor divided by the extent.
        //
        // Two floors, because a split narrower than twice the preferred one
        // cannot honour it — and clamping to it anyway pinned the gutter at
        // dead centre, so dragging it did nothing at all and read as broken.
        // Where the room is not there, what is defended instead is only that
        // both sides stay wide enough to keep their corner button reachable:
        // a seat squeezed to nothing is a seat that cannot be put back.
        let wanted = if *columns { SEAT_MIN_W } else { SEAT_MIN_H };
        let floor = if wanted * 2.0 <= extent {
            wanted
        } else {
            SEAT_MIN_GRAB.min(extent / 2.0)
        } / extent;
        *ratio = (from + delta / extent).clamp(floor, 1.0 - floor);
    }));

    // ── tabs ──
    editor.on_tab_selected(on_timeline!(|state, index: i32| {
        if index >= 0 && (index as usize) < state.timelines.len() {
            state.active = index as usize;
            // A selection is a set of clip ids on *this* timeline; carrying it
            // across would leave the tray counting clips nobody can see.
            state.selection.clear();
        }
    }));

    editor.on_tab_renamed(on_timeline!(|state, index: i32, name: SharedString| {
        let trimmed = name.trim().to_string();
        // Renames to whitespace are ignored, so a tab is never blank.
        if !trimmed.is_empty()
            && let Some(timeline) = state.timelines.get_mut(index as usize)
        {
            timeline.name = trimmed;
        }
    }));

    editor.on_tab_added(on_timeline!(|state| {
        let id = state.mint("TL");
        let lane = state.mint("T");
        let number = state.timelines.len() + 1;
        state.timelines.push(TimelineDoc {
            id,
            name: format!("Timeline {number}"),
            tracks: vec![track(&lane, "Track 1")],
            clips: Vec::new(),
        });
        state.active = state.timelines.len() - 1;
        state.selection.clear();
    }));

    // Deleting a timeline throws away every clip on it. The reference asks
    // first, through a confirm dialog this tree has not grown yet — so the
    // floor of one timeline is enforced and the confirm is the gap.
    editor.on_tab_close_requested(on_timeline!(|state, index: i32| {
        if state.timelines.len() > 1 && (index as usize) < state.timelines.len() {
            state.timelines.remove(index as usize);
            state.active = state.active.min(state.timelines.len() - 1);
            state.selection.clear();
        }
    }));

    // ── the tray ──
    editor.on_tool_changed(on_timeline!(|state, tool: TimelineTool| {
        state.tool = tool;
    }));

    editor.on_snap_changed(on_timeline!(|state, snap: bool| {
        state.snap = snap;
    }));

    editor.on_add_track(on_timeline!(|state| {
        let id = state.mint("T");
        let name = state.next_track_name();
        // Pushed, not inserted: the model runs bottom-up, so the end of the
        // list is the top of the stack — which is where a new lane belongs.
        state.now_mut().tracks.push(track(&id, &name));
    }));

    editor.on_delete_selected(on_timeline!(|state| {
        let doomed = state.selection.clone();
        state
            .now_mut()
            .clips
            .retain(|clip| !doomed.contains(&clip.id));
        state.selection.clear();
    }));

    // Split at the playhead: every selected clip the playhead runs through, or
    // every clip at all when nothing is selected — which is what a split with
    // no selection means in an editor.
    editor.on_split(on_timeline!(|state| {
        let at = state.playhead;
        let selection = state.selection.clone();
        let victims: Vec<String> = state
            .now()
            .clips
            .iter()
            .filter(|clip| {
                clip.start + MIN_DURATION < at
                    && at < clip.end() - MIN_DURATION
                    && (selection.is_empty() || selection.contains(&clip.id))
                    && !state.locked(&clip.track)
            })
            .map(|clip| clip.id.clone())
            .collect();

        for victim in victims {
            let Some(index) = state.now().clips.iter().position(|clip| clip.id == victim) else {
                continue;
            };
            let source = state.now().clips[index].clone();
            let new_id = state.mint("c");
            let head_duration = at - source.start;

            // The head keeps the id, the way the reference's split does, so a
            // command naming the clip still names something afterwards. The
            // ramps do not survive the cut: a fade-out belongs to the end of
            // the material, and the head no longer has one.
            let mut tail = source.clone();
            tail.id = new_id;
            tail.start = at;
            tail.duration = source.duration - head_duration;
            tail.source_start = source.source_start + head_duration;
            tail.fade_in = 0.0;
            tail.transition_in = false;

            let head = &mut state.now_mut().clips[index];
            head.duration = head_duration;
            head.fade_out = 0.0;
            state.now_mut().clips.push(tail);
        }
    }));

    editor.on_merge(on_timeline!(|state| {
        if state.merge_blocked().is_some() {
            return;
        }
        let (a, b) = (state.selection[0].clone(), state.selection[1].clone());
        let (Some(first), Some(second)) = (
            state.now().clips.iter().position(|clip| clip.id == a),
            state.now().clips.iter().position(|clip| clip.id == b),
        ) else {
            return;
        };
        let (head, tail) = if state.now().clips[first].start <= state.now().clips[second].start {
            (first, second)
        } else {
            (second, first)
        };
        let tail_clip = state.now().clips[tail].clone();
        let head_clip = &mut state.now_mut().clips[head];
        head_clip.duration = tail_clip.end() - head_clip.start;
        head_clip.fade_out = tail_clip.fade_out;
        let kept = head_clip.id.clone();
        state.now_mut().clips.remove(tail);
        state.selection = vec![kept];
    }));

    // ── the view ──
    editor.on_scrubbed(on_lanes!(|state, seconds: f32| {
        state.playhead = seconds.max(0.0);
    }));

    editor.on_scrolled(on_lanes!(|state, seconds: f32| {
        state.scroll_left = seconds.max(0.0);
    }));

    editor.on_zoom(on_lanes!(|state, factor: f32, anchor: f32| {
        let before = state.seconds_per_pixel;
        // Floored at a frame across two pixels and capped at ten minutes to
        // the pane, which is as far either way as the ruler stays legible.
        let after = (before * factor).clamp(0.000_5, 1.5);
        state.seconds_per_pixel = after;
        // Keep the moment under the pointer where it is. A press has no
        // pointer, and passes -1 to mean "hold the left edge".
        if anchor >= 0.0 {
            state.scroll_left = (anchor - (anchor - state.scroll_left) * (after / before)).max(0.0);
        }
    }));

    editor.on_zoom_to_fit(on_lanes!(|state, width: f32| {
        // A little air past the end, so the last clip is not flush against the
        // right edge, and a floor so an empty timeline does not divide by zero.
        let span = state.duration().max(1.0) * 1.05;
        if width > 1.0 {
            state.seconds_per_pixel = (span / width).clamp(0.000_5, 1.5);
            state.scroll_left = 0.0;
        }
    }));

    // ── lanes ──
    editor.on_track_flag_changed(on_timeline!(
        |state, row: i32, visible: bool, muted: bool, locked: bool| {
            let Some(id) = state.row_track(row).map(|track| track.id.clone()) else {
                return;
            };
            if let Some(track) = state
                .now_mut()
                .tracks
                .iter_mut()
                .find(|track| track.id == id)
            {
                track.visible = visible;
                track.muted = muted;
                track.locked = locked;
            }
            // Locking a lane drops whatever was selected on it, so the tray
            // cannot offer to delete clips that no longer answer to it.
            if locked {
                let doomed: Vec<String> = state
                    .now()
                    .clips
                    .iter()
                    .filter(|clip| clip.track == id)
                    .map(|clip| clip.id.clone())
                    .collect();
                state.selection.retain(|held| !doomed.contains(held));
            }
        }
    ));

    editor.on_track_renamed(on_timeline!(|state, row: i32, name: SharedString| {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        let Some(id) = state.row_track(row).map(|track| track.id.clone()) else {
            return;
        };
        if let Some(track) = state
            .now_mut()
            .tracks
            .iter_mut()
            .find(|track| track.id == id)
        {
            track.name = trimmed;
        }
    }));

    // Removing a lane takes its clips with it, and keeps a floor of one.
    editor.on_track_sized(on_timeline!(|state, row: i32, size: TrackSize| {
        let Some(id) = state.row_track(row).map(|lane| lane.id.clone()) else {
            return;
        };
        if let Some(lane) = state.now_mut().tracks.iter_mut().find(|lane| lane.id == id) {
            lane.size = size;
        }
    }));

    editor.on_track_removed(on_timeline!(|state, row: i32| {
        if state.now().tracks.len() <= 1 {
            return;
        }
        let Some(id) = state.row_track(row).map(|track| track.id.clone()) else {
            return;
        };
        state.now_mut().tracks.retain(|track| track.id != id);
        state.now_mut().clips.retain(|clip| clip.track != id);
        let survivors: Vec<String> = state
            .now()
            .clips
            .iter()
            .map(|clip| clip.id.clone())
            .collect();
        state.selection.retain(|id| survivors.contains(id));
    }));

    // ── the gestures ──
    //
    // A press resolves the selection before anything moves, so a drag that
    // starts on an already-selected clip carries the whole set rather than
    // collapsing to the one clip touched.
    editor.on_clip_pressed(on_lanes!(|state,
                                      id: SharedString,
                                      additive: bool,
                                      edge: i32| {
        let id = id.to_string();
        let Some(clip) = state.clip(&id).cloned() else {
            return;
        };
        // A locked lane takes no presses at all — not even to select. Half a
        // lock, where a clip can still be picked up and then deleted from the
        // tray, is worse than none.
        if state.locked(&clip.track) {
            return;
        }
        let already = state.selection.contains(&id);

        state.selection = if additive {
            if already {
                state
                    .selection
                    .iter()
                    .filter(|held| *held != &id)
                    .cloned()
                    .collect()
            } else {
                let mut next = state.selection.clone();
                next.push(id.clone());
                next
            }
        } else if already {
            state.selection.clone()
        } else {
            vec![id.clone()]
        };

        // An edge only trims while it is the one thing selected: dragging the
        // edge of a multiple selection is a move of the set, not a trim.
        if edge >= 0 && state.selection.len() <= 1 {
            state.gesture = Gesture::Trim {
                clip: id,
                edge: if edge == 0 { Edge::Start } else { Edge::End },
                start: clip.start,
                duration: clip.duration,
                source_start: clip.source_start,
            };
            return;
        }

        let moving = if state.selection.contains(&id) {
            state.selection.clone()
        } else {
            vec![id.clone()]
        };
        let origins = moving
            .iter()
            .filter_map(|clip_id| {
                let clip = state.clip(clip_id)?;
                Some(MoveOrigin {
                    clip: clip.id.clone(),
                    start: clip.start,
                    row: state.row_of(&clip.track),
                })
            })
            .collect();
        state.gesture = Gesture::Move {
            primary: id,
            origins,
            lanes: state.lane_heights(),
        };
    }));

    editor.on_clip_dragged(on_lanes!(|state, seconds: f32, pixels: f32| {
        // Lifted out rather than matched in place: every arm goes on to mutate
        // the clips, which cannot happen while the gesture is borrowed out of
        // the same struct. It goes back at the end, so a drag survives.
        let gesture = std::mem::replace(&mut state.gesture, Gesture::None);
        match &gesture {
            Gesture::Move {
                primary,
                origins,
                lanes,
            } => {
                let Some(anchor) = origins.iter().find(|origin| &origin.clip == primary) else {
                    return;
                };
                // Only the grabbed clip snaps; the rest keep their offset from
                // it, so the shape of a multiple selection survives exactly.
                let threshold = 8.0 * state.seconds_per_pixel;
                let snapped = state.snapped(anchor.start + seconds, threshold, primary);
                let shift = snapped - anchor.start;

                // The lanes are no longer one pitch, so the row cannot be a
                // division: it is the row whose top edge the grabbed clip's
                // has been dragged nearest to, measured against the stack as
                // it was when the press landed. The rest of the selection
                // keeps its offset from that in rows.
                let rows = nearest_row(lanes, row_top(lanes, anchor.row) + pixels) - anchor.row;

                let count = state.now().tracks.len() as i32;
                let moves: Vec<(String, f32, Option<String>)> = origins
                    .iter()
                    .map(|origin| {
                        let row = (origin.row + rows).clamp(0, count - 1);
                        // A locked lane refuses the drop: the clip still moves
                        // in time, it just stays on the lane it came from. The
                        // reference does the same for an unknown track id.
                        let onto = state
                            .row_track(row)
                            .filter(|track| !track.locked)
                            .map(|track| track.id.clone());
                        (origin.clip.clone(), (origin.start + shift).max(0.0), onto)
                    })
                    .collect();

                for (id, start, track) in moves {
                    if let Some(clip) = state.now_mut().clips.iter_mut().find(|clip| clip.id == id)
                    {
                        clip.start = start;
                        if let Some(track) = track {
                            clip.track = track;
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
                let threshold = 8.0 * state.seconds_per_pixel;

                if edge == Edge::Start {
                    // The head cannot pass the tail, and cannot pull material
                    // out of a file that has none before the in-point.
                    let wanted = state.snapped(start + seconds, threshold, &id);
                    let limit = start + duration - MIN_DURATION;
                    let at = wanted.clamp((start - source_start).max(0.0), limit);
                    let delta = at - start;
                    if let Some(clip) = state.now_mut().clips.iter_mut().find(|c| c.id == id) {
                        clip.start = at;
                        clip.duration = duration - delta;
                        clip.source_start = source_start + delta;
                    }
                } else {
                    let wanted = state.snapped(start + duration + seconds, threshold, &id);
                    let at = wanted.max(start + MIN_DURATION);
                    if let Some(clip) = state.now_mut().clips.iter_mut().find(|c| c.id == id) {
                        clip.duration = at - start;
                    }
                }
            }
            Gesture::None => {}
        }
        state.gesture = gesture;
    }));

    // Releasing is what would make a move or a trim real: the reference echoes
    // it locally and turns the whole gesture into one undoable command here.
    // There is no command stack in this tree, so this only ends the gesture.
    editor.on_clip_released(on_lanes!(|state| {
        state.gesture = Gesture::None;
    }));

    // ── drag and drop from the library ──
    //
    // Two callbacks over one resolver. The hover asks what the drop would
    // leave and publishes the answer as the ghost the lanes draw; the drop
    // asks the same question again and commits it. Asking twice rather than
    // trusting the last hover is deliberate: the release carries its own
    // position, and a drop that landed a pixel from where the ghost was would
    // be a bug nobody could see.
    editor.on_drag_hovered({
        let weak = app.as_weak();
        let studio = studio.clone();
        let models = models.clone();
        let library = library.clone();
        move |payload: SharedString, seconds: f32, y: f32| {
            let Some(app) = weak.upgrade() else { return };
            {
                let mut state = studio.borrow_mut();
                let row = state.row_at(y);
                state.drop = state.plan(payload.as_str(), seconds, row, &library);
            }
            // The lanes only: this runs on every move of the drag.
            studio.borrow().publish_lanes(&app, &models);
        }
    });

    editor.on_dropped({
        let weak = app.as_weak();
        let studio = studio.clone();
        let models = models.clone();
        let library = library.clone();
        move |payload: SharedString, seconds: f32, y: f32| {
            let Some(app) = weak.upgrade() else { return };
            {
                let mut state = studio.borrow_mut();
                state.drop = None;
                let row = state.row_at(y);
                if let Some(plan) = state.plan(payload.as_str(), seconds, row, &library) {
                    state.place(&plan);
                }
            }
            studio.borrow().publish(&app, &models);
        }
    });

    // ── the same three, placed by a click ──
    //
    // Every card is still clickable, and a click has no pointer over the
    // lanes to name a place — so these land at the playhead, on the topmost
    // lane with room for them.
    editor.on_media_activate({
        let weak = app.as_weak();
        let studio = studio.clone();
        let models = models.clone();
        let library = library.clone();
        move |id: i32| {
            let Some(app) = weak.upgrade() else { return };
            {
                let mut state = studio.borrow_mut();
                if let Some(plan) = state.plan_at_playhead(&format!("media:{id}"), &library) {
                    state.place(&plan);
                }
            }
            studio.borrow().publish(&app, &models);
        }
    });

    editor.on_library_add_text({
        let weak = app.as_weak();
        let studio = studio.clone();
        let models = models.clone();
        let library = library.clone();
        move || {
            let Some(app) = weak.upgrade() else { return };
            {
                let mut state = studio.borrow_mut();
                if let Some(plan) = state.plan_at_playhead("text:default:Title", &library) {
                    state.place(&plan);
                }
            }
            studio.borrow().publish(&app, &models);
        }
    });

    editor.on_library_apply_filter({
        let weak = app.as_weak();
        let studio = studio.clone();
        let models = models.clone();
        let library = library.clone();
        move |id: SharedString, label: SharedString| {
            let Some(app) = weak.upgrade() else { return };
            {
                let mut state = studio.borrow_mut();
                let payload = format!("filter:{id}:{label}");
                if let Some(plan) = state.plan_at_playhead(&payload, &library) {
                    state.place(&plan);
                }
            }
            studio.borrow().publish(&app, &models);
        }
    });

    // An effect is not placed: it is a treatment of one clip, and the model
    // carries exactly one fact about that — whether any are live. Which
    // effect it was needs the applied-effect chain the inspector's stack is
    // drawn for, and nothing on this side owns one yet.
    editor.on_library_apply_effect(on_timeline!(|state, _id: SharedString| {
        // The two refusals the reference explains rather than swallows. A
        // card that does nothing when clicked and says nothing about it is
        // the one failure mode a catalogue cannot afford: there is no way to
        // tell it from a card that is broken.
        let Some(id) = state.selection.first().cloned() else {
            state.notify("Select a video or image clip on the timeline first", true);
            return;
        };
        let has_picture = state
            .clip(&id)
            .is_some_and(|clip| matches!(clip.kind, ClipKind::Video | ClipKind::Image));
        if !has_picture {
            state.notify("Select a video or image clip on the timeline first", true);
            return;
        }
        if let Some(clip) = state.now_mut().clips.iter_mut().find(|clip| clip.id == id) {
            clip.fx = true;
        }
    }));

    editor.on_razored(on_timeline!(|state, id: SharedString, seconds: f32| {
        let id = id.to_string();
        let Some(index) = state.now().clips.iter().position(|clip| clip.id == id) else {
            return;
        };
        let source = state.now().clips[index].clone();
        if state.locked(&source.track) {
            return;
        }
        if seconds <= source.start + MIN_DURATION || seconds >= source.end() - MIN_DURATION {
            return;
        }
        let new_id = state.mint("c");
        let head_duration = seconds - source.start;
        let mut tail = source.clone();
        tail.id = new_id;
        tail.start = seconds;
        tail.duration = source.duration - head_duration;
        tail.source_start = source.source_start + head_duration;
        tail.fade_in = 0.0;
        tail.transition_in = false;

        let head = &mut state.now_mut().clips[index];
        head.duration = head_duration;
        head.fade_out = 0.0;
        state.now_mut().clips.push(tail);
        state.selection = vec![id];
    }));

    editor.on_band_selected(on_lanes!(
        |state, from: f32, to: f32, from_y: f32, to_y: f32, additive: bool| {
            // The band arrives as two corners in pixels down the stack, and
            // the rows are resolved here for the reason the drops are: the
            // lanes are no longer one height, and this side is the one that
            // knows what each of them is.
            let (from_row, to_row) = (state.row_at(from_y), state.row_at(to_y));
            let caught: Vec<String> = state
                .now()
                .clips
                .iter()
                .filter(|clip| {
                    let row = state.row_of(&clip.track);
                    row >= from_row
                        && row <= to_row
                        && clip.end() >= from
                        && clip.start <= to
                        && !state.locked(&clip.track)
                })
                .map(|clip| clip.id.clone())
                .collect();

            if additive {
                for id in caught {
                    if !state.selection.contains(&id) {
                        state.selection.push(id);
                    }
                }
            } else {
                state.selection = caught;
            }
        }
    ));

    // ── the inspector ──
    //
    // One field and one number rather than a patch of the whole clip: a patch
    // is "everything, with one thing different", and a late-arriving one
    // overwrites whatever landed while it was in flight.
    editor.on_clip_set(on_lanes!(|state, field: ClipField, value: f32| {
        let Some(id) = state.selection.first().cloned() else {
            return;
        };
        let Some(index) = state.now().clips.iter().position(|clip| clip.id == id) else {
            return;
        };
        // Speed re-times the clip in place, so it needs the old rate to scale
        // the duration by; read before the write.
        let previous_speed = state.now().clips[index].speed;
        let clip = &mut state.now_mut().clips[index];

        match field {
            ClipField::Scale => clip.scale = value.clamp(0.05, 8.0),
            ClipField::OffsetX => clip.offset_x = value.clamp(-1.0, 1.0),
            ClipField::OffsetY => clip.offset_y = value.clamp(-1.0, 1.0),
            ClipField::Rotation => clip.rotation = value.clamp(-180.0, 180.0),
            ClipField::Opacity => clip.opacity = value.clamp(0.0, 1.0),
            ClipField::Volume => clip.volume = value.max(0.0),
            ClipField::Speed => {
                let speed = value.clamp(0.0625, 16.0);
                // Source seconds are fixed; the timeline seconds they occupy
                // are what changes. Without this the clip would keep its
                // length and simply play a different amount of material.
                clip.duration = (clip.duration * previous_speed / speed).max(MIN_DURATION);
                clip.speed = speed;
            }
            ClipField::PreservePitch => clip.preserve_pitch = value != 0.0,
            // A fade longer than half the clip would overlap the other one.
            ClipField::FadeIn => clip.fade_in = value.clamp(0.0, clip.duration / 2.0),
            ClipField::FadeOut => clip.fade_out = value.clamp(0.0, clip.duration / 2.0),

            ClipField::FontSize => clip.text.size = value.clamp(0.01, 0.5),
            ClipField::FontWeight => clip.text.weight = value.clamp(100.0, 900.0),
            ClipField::Italic => clip.text.italic = value != 0.0,
            ClipField::TextOpacity => clip.text.opacity = value.clamp(0.0, 1.0),
            ClipField::Align => {
                clip.text.align = match value as i32 {
                    0 => TextAlignment::Left,
                    2 => TextAlignment::Right,
                    _ => TextAlignment::Center,
                }
            }
            ClipField::StrokeWidth => clip.text.stroke_width = value.clamp(0.0, 0.15),
            ClipField::Shadow => clip.text.shadow = value != 0.0,
            ClipField::LineHeight => clip.text.line_height = value.clamp(0.7, 2.5),
            ClipField::Tracking => clip.text.tracking = value.clamp(-0.05, 0.3),
        }
    }));

    editor.on_clip_set_text(on_lanes!(
        |state, field: ClipTextField, value: SharedString| {
            let Some(id) = state.selection.first().cloned() else {
                return;
            };
            let Some(index) = state.now().clips.iter().position(|clip| clip.id == id) else {
                return;
            };
            let clip = &mut state.now_mut().clips[index];
            match field {
                ClipTextField::Content => {
                    clip.text_body = value.to_string();
                    // The clip's label follows its first line, the way the
                    // reference snapshots a title's words into its name — a lane
                    // full of clips all called "Title" is a lane you cannot read.
                    let first = value.lines().next().unwrap_or("").trim().to_string();
                    clip.name = if first.is_empty() {
                        "Title".into()
                    } else {
                        first
                    };
                }
                ClipTextField::FontFamily => clip.text.family = value.to_string(),
                // The colours arrive through `clip-set-colour`; a colour cannot be
                // spelled as a string Slint could parse back.
                _ => {}
            }
        }
    ));

    editor.on_clip_set_colour(on_lanes!(
        |state, field: ClipTextField, value: slint::Color| {
            let Some(id) = state.selection.first().cloned() else {
                return;
            };
            let Some(index) = state.now().clips.iter().position(|clip| clip.id == id) else {
                return;
            };
            let clip = &mut state.now_mut().clips[index];
            match field {
                ClipTextField::Color => clip.text.fill = value,
                ClipTextField::StrokeColor => clip.text.stroke = value,
                ClipTextField::Background => clip.text.plate = value,
                _ => {}
            }
        }
    ));

    // The gesture is over. The reference turns everything it accumulated into
    // one undoable command here; there is no command stack in this tree yet,
    // so the seam is the callback.
    editor.on_clip_commit(|| {});

    // ── the monitor ──
    //
    // The transport moves the timeline's playhead rather than a second copy of
    // it: the programme position is one number, and a monitor that kept its
    // own would drift from the lanes the moment either moved.
    editor.on_seek(on_lanes!(|state, seconds: f32| {
        state.playing = false;
        state.playhead = seconds.clamp(0.0, state.duration());
    }));

    editor.on_step_frames(on_lanes!(|state, frames: f32| {
        state.playing = false;
        // Stepped on the frame grid, not by adding a fraction of a second:
        // repeated steps off-grid would accumulate a drift that shows up as a
        // frame field that skips a number.
        let fps = state.frame_rate.round().max(1.0);
        let at = (state.playhead * fps).round() + frames;
        state.playhead = (at / fps).clamp(0.0, state.duration());
    }));

    editor.on_ratio_changed(on_timeline!(|state, index: i32| {
        state.ratio = (index.max(0) as usize).min(OUTPUTS.len() - 1);
    }));

    // Preview quality: what fraction of the frame the engine composites for
    // the monitor. It changes nothing on screen here — nothing composites yet
    // — but it is a project setting rather than a view one, so it lives with
    // the rest of them.
    editor.on_quality_changed(on_timeline!(|state, index: i32| {
        state.quality = (index.max(0) as usize).min(2);
    }));

    // Playback.
    //
    // A timer only while it is playing, and stopped the moment it is not. The
    // level meter next door had to be gated on hover because it ran forever;
    // this one is self-limiting — a transport that is not running produces no
    // frames, and reaching the tail stops it.
    let playback = Rc::new(Timer::default());
    editor.on_play_toggled({
        let weak = app.as_weak();
        let studio = studio.clone();
        let models = models.clone();
        let playback = playback.clone();
        move || {
            let Some(app) = weak.upgrade() else { return };
            {
                let mut state = studio.borrow_mut();
                if state.playing {
                    state.playing = false;
                    playback.stop();
                } else {
                    // Playing from the tail would sit there doing nothing, so
                    // the button rewinds first — what every editor does.
                    if state.playhead >= state.duration() {
                        state.playhead = 0.0;
                    }
                    state.playing = true;
                }
            }
            let running = studio.borrow().playing;
            if running {
                let rate = studio.borrow().frame_rate.round().max(1.0);
                playback.start(
                    TimerMode::Repeated,
                    std::time::Duration::from_secs_f32(1.0 / rate),
                    {
                        let weak = weak.clone();
                        let studio = studio.clone();
                        let models = models.clone();
                        let playback = playback.clone();
                        move || {
                            let Some(app) = weak.upgrade() else { return };
                            {
                                let mut state = studio.borrow_mut();
                                let end = state.duration();
                                let step = 1.0 / state.frame_rate.round().max(1.0);
                                state.playhead += step;
                                if state.playhead >= end {
                                    state.playhead = end;
                                    state.playing = false;
                                }
                            }
                            if !studio.borrow().playing {
                                playback.stop();
                            }
                            // A frame of playback moves the playhead and nothing
                            // else — no menu, no dialog, no engine list.
                            studio.borrow().publish_lanes(&app, &models);
                        }
                    },
                );
            }
            studio.borrow().publish(&app, &models);
        }
    });

    // ── the context menu ──
    editor.on_clip_context(on_timeline!(|state, id: SharedString| {
        let id = id.to_string();
        // The token moves either way, even when there is nothing to show.
        //
        // The panel that right-clicked arms itself before calling, and
        // disarms when the token it is waiting for arrives — so a request
        // that quietly declined to bump left it armed, and the *next* one,
        // from a different timeline seat, opened a second menu here at
        // whatever corner this pane was last pointed at. An empty list draws
        // no menu (see ContextMenu), so bumping regardless costs nothing.
        state.menu_token += 1;
        if state.clip(&id).is_none() {
            state.menu_target = None;
            return;
        }
        // Right-clicking selects, the way the reference does: the menu acts on
        // one clip and the selection should say which one before it does.
        if !state.selection.contains(&id) {
            state.selection = vec![id.clone()];
        }
        state.menu_target = Some(id);
    }));

    editor.on_menu_selected(on_timeline!(|state, action: SharedString| {
        let Some(id) = state.menu_target.clone() else {
            return;
        };
        let Some(index) = state.now().clips.iter().position(|clip| clip.id == id) else {
            return;
        };
        let clip = state.now().clips[index].clone();

        match action.as_str() {
            "copy" => state.clipboard = Some(clip),
            "duplicate" | "paste" => {
                let source = if action == "paste" {
                    let Some(held) = state.clipboard.clone() else {
                        return;
                    };
                    held
                } else {
                    clip
                };
                let new_id = state.mint("c");
                let mut copy = source.clone();
                copy.id = new_id;
                // Laid after the clip it came from rather than on top of it: a
                // duplicate hidden exactly behind its original looks like
                // nothing happened.
                copy.start = source.end();
                copy.name = format!("{} copy", source.name);
                state.now_mut().clips.push(copy);
            }
            "split" => {
                let at = state.playhead;
                if clip.start + MIN_DURATION >= at || at >= clip.end() - MIN_DURATION {
                    return;
                }
                let new_id = state.mint("c");
                let head = at - clip.start;
                let mut tail = clip.clone();
                tail.id = new_id;
                tail.start = at;
                tail.duration = clip.duration - head;
                tail.source_start = clip.source_start + head;
                tail.fade_in = 0.0;
                tail.transition_in = false;
                state.now_mut().clips[index].duration = head;
                state.now_mut().clips[index].fade_out = 0.0;
                state.now_mut().clips.push(tail);
            }
            "mute" => {
                // Restored to unity rather than to whatever it was: the menu
                // holds no memory, and a mute that came back at -14 dB would be
                // a surprise every time.
                let target = &mut state.now_mut().clips[index];
                target.volume = if target.volume <= 0.0 { 1.0 } else { 0.0 };
            }
            "lock" => {
                let track = clip.track.clone();
                let now = state.locked(&track);
                if let Some(lane) = state.now_mut().tracks.iter_mut().find(|t| t.id == track) {
                    lane.locked = !now;
                }
                if !now {
                    let doomed: Vec<String> = state
                        .now()
                        .clips
                        .iter()
                        .filter(|c| c.track == track)
                        .map(|c| c.id.clone())
                        .collect();
                    state.selection.retain(|held| !doomed.contains(held));
                }
            }
            "delete" => {
                state.now_mut().clips.retain(|c| c.id != id);
                state.selection.retain(|held| held != &id);
                state.menu_target = None;
            }
            _ => {}
        }
    }));

    // ── the dialogs ──
    app.on_export_clicked(on_timeline!(|state| {
        state.export.open = true;
        state.export.phase = ExportPhase::Idle;
        state.export.message = String::new();
    }));
    app.on_open_settings(on_timeline!(|state| {
        state.settings.open = true;
    }));
    // The theme is not editor state and never reaches a project file: it is
    // one bool on the Theme global, and every colour in the tree is a binding
    // away from it, so setting it here retones the window on the next frame
    // without anything being rebuilt or republished. Handled in Rust rather
    // than in the .slint so there is somewhere for a remembered preference to
    // be read and written the day there is a place to keep one.
    app.on_settings_theme_changed({
        let weak = app.as_weak();
        move |dark| {
            let Some(app) = weak.upgrade() else { return };
            app.global::<Theme>().set_dark(dark);
        }
    });
    app.on_export_closed(on_timeline!(|state| {
        state.export.open = false;
    }));
    app.on_settings_closed(on_timeline!(|state| {
        state.settings.open = false;
    }));

    app.on_export_name_edited(on_timeline!(|state, name: SharedString| {
        state.export.name = name.to_string();
    }));
    app.on_export_resolution_changed(on_timeline!(|state, index: i32| {
        state.export.resolution = (index.max(0) as usize).min(3);
    }));
    app.on_export_rate_changed(on_timeline!(|state, index: i32| {
        state.export.rate = (index.max(0) as usize).min(2);
    }));
    app.on_export_quality_changed(on_timeline!(|state, index: i32| {
        state.export.quality = (index.max(0) as usize).min(2);
    }));
    app.on_export_again(on_timeline!(|state| {
        state.export.phase = ExportPhase::Idle;
        state.export.progress = 0.0;
    }));
    // Both of these want a file manager, which is a dependency decision rather
    // than a detail of this dialog. The seams are here for the day it lands.
    app.on_export_browse(|| {});
    app.on_export_reveal(|| {});

    // The run itself. A timer, like playback: it exists only while the job
    // does, and it stops itself at the end.
    let exporting = Rc::new(Timer::default());
    app.on_export_cancel({
        let weak = app.as_weak();
        let studio = studio.clone();
        let models = models.clone();
        let exporting = exporting.clone();
        move || {
            let Some(app) = weak.upgrade() else { return };
            exporting.stop();
            let mut state = studio.borrow_mut();
            state.export.phase = ExportPhase::Idle;
            state.export.progress = 0.0;
            drop(state);
            studio.borrow().publish(&app, &models);
        }
    });

    app.on_export_start({
        let weak = app.as_weak();
        let studio = studio.clone();
        let models = models.clone();
        let exporting = exporting.clone();
        move || {
            let Some(app) = weak.upgrade() else { return };
            {
                let mut state = studio.borrow_mut();
                state.export.phase = ExportPhase::Running;
                state.export.progress = 0.0;
                state.export.message = String::new();
            }
            exporting.start(TimerMode::Repeated, std::time::Duration::from_millis(90), {
                let weak = weak.clone();
                let studio = studio.clone();
                let models = models.clone();
                let exporting = exporting.clone();
                move || {
                    let Some(app) = weak.upgrade() else { return };
                    {
                        let mut state = studio.borrow_mut();
                        state.export.progress = (state.export.progress + 0.012).min(1.0);
                        if state.export.progress >= 1.0 {
                            state.export.phase = ExportPhase::Done;
                        }
                    }
                    if studio.borrow().export.phase != ExportPhase::Running {
                        exporting.stop();
                    }
                    studio.borrow().publish(&app, &models);
                }
            });
            studio.borrow().publish(&app, &models);
        }
    });

    // ── settings ──
    app.on_settings_page_changed(on_timeline!(|state, index: i32| {
        state.settings.tab = index;
    }));
    app.on_settings_language_changed(on_timeline!(|state, index: i32| {
        state.settings.language = index.max(0) as usize;
    }));
    app.on_settings_transcribe_language_changed(on_timeline!(|state, index: i32| {
        state.settings.transcribe_language = index;
    }));

    app.on_model_activated(on_timeline!(|state, id: SharedString| {
        // One active model per engine, so choosing is also un-choosing. Which
        // list the id is in decides which engine it belongs to.
        let engines = &mut *state;
        for list in [&mut engines.transcribers, &mut engines.voices] {
            if list
                .iter()
                .any(|model| model.id == id.as_str() && model.installed)
            {
                for model in list.iter_mut() {
                    model.active = model.id == id.as_str();
                }
            }
        }
    }));

    app.on_model_download(on_timeline!(|state, id: SharedString| {
        let engines = &mut *state;
        for list in [&mut engines.transcribers, &mut engines.voices] {
            if let Some(model) = list.iter_mut().find(|model| model.id == id.as_str()) {
                model.fetched = Some(0.0);
            }
        }
    }));

    app.on_model_cancel(on_timeline!(|state, id: SharedString| {
        let engines = &mut *state;
        for list in [&mut engines.transcribers, &mut engines.voices] {
            if let Some(model) = list.iter_mut().find(|model| model.id == id.as_str()) {
                model.fetched = None;
            }
        }
    }));

    app.on_model_remove(on_timeline!(|state, id: SharedString| {
        let engines = &mut *state;
        for list in [&mut engines.transcribers, &mut engines.voices] {
            if let Some(model) = list.iter_mut().find(|model| model.id == id.as_str()) {
                model.installed = false;
                model.active = false;
                model.fetched = None;
            }
            // An engine with nothing active falls back to whatever is left,
            // rather than silently having no model at all.
            if list.iter().all(|model| !model.active)
                && let Some(next) = list.iter_mut().find(|model| model.installed)
            {
                next.active = true;
            }
        }
    }));

    // Downloads advance on one timer for all of them: they are simulated, and
    // a timer each would be a timer each for no reason.
    let downloads = Rc::new(Timer::default());
    downloads.start(
        TimerMode::Repeated,
        std::time::Duration::from_millis(120),
        {
            let weak = app.as_weak();
            let studio = studio.clone();
            let models = models.clone();
            move || {
                let Some(app) = weak.upgrade() else { return };
                let mut moved = false;
                {
                    let mut state = studio.borrow_mut();
                    let engines = &mut *state;
                    for list in [&mut engines.transcribers, &mut engines.voices] {
                        for model in list.iter_mut() {
                            let Some(fetched) = model.fetched else {
                                continue;
                            };
                            moved = true;
                            let next = fetched + 12.0 * 0.12 * 8.0;
                            if next >= model.megabytes {
                                model.fetched = None;
                                model.installed = true;
                            } else {
                                model.fetched = Some(next);
                            }
                        }
                    }
                }
                // Only republish when something actually moved: this ticks for the
                // life of the process, and a republish on every tick would redraw
                // the whole window eight times a second to change nothing.
                if moved {
                    studio.borrow().publish(&app, &models);
                }
            }
        },
    );

    // ── the tray's A/V menu ──
    editor.on_av_tools(on_timeline!(|state| {
        state.av_token += 1;
    }));

    editor.on_av_selected(on_timeline!(|state, action: SharedString| {
        let Some(id) = state.selection.first().cloned() else {
            return;
        };
        let Some(index) = state.now().clips.iter().position(|clip| clip.id == id) else {
            return;
        };
        let clip = state.now().clips[index].clone();

        match action.as_str() {
            // Both of these end in a dialog this tree has not grown — the
            // transcriber's sheet and the speech one. The edit each performs is
            // real; what is missing is the sheet in front of it.
            "captions" | "speak" => {}
            "detach" => {
                // The sound leaves the picture and becomes its own clip on the
                // lane below, keeping a pointer home so it can be put back.
                let new_id = state.mint("c");
                let row = state.row_of(&clip.track);
                let below = state
                    .row_track(row + 1)
                    .map(|track| track.id.clone())
                    .unwrap_or_else(|| clip.track.clone());
                let mut sound = clip.clone();
                sound.id = new_id;
                sound.kind = ClipKind::Audio;
                sound.track = below;
                sound.name = format!("{} audio", clip.name);
                sound.detached_from = Some(clip.id.clone());
                sound.fx = false;
                sound.transition_in = false;
                state.now_mut().clips[index].volume = 0.0;
                state.now_mut().clips.push(sound);
            }
            "reattach" => {
                let parent = clip
                    .detached_from
                    .clone()
                    .unwrap_or_else(|| clip.id.clone());
                let doomed: Vec<String> = state
                    .now()
                    .clips
                    .iter()
                    .filter(|other| other.detached_from.as_deref() == Some(parent.as_str()))
                    .map(|other| other.id.clone())
                    .collect();
                state
                    .now_mut()
                    .clips
                    .retain(|other| !doomed.contains(&other.id));
                if let Some(video) = state.now_mut().clips.iter_mut().find(|c| c.id == parent) {
                    video.volume = 1.0;
                }
                state.selection = vec![parent];
            }
            _ => {}
        }
    }));

    // ── the title-bar menus ──
    app.on_menu_opened(on_timeline!(|state, index: i32| {
        state.open_menu = index;
        state.menu_bar_token += 1;
    }));

    app.on_app_menu_selected({
        let weak = app.as_weak();
        let studio = studio.clone();
        let models = models.clone();
        move |action| {
            let Some(app) = weak.upgrade() else { return };
            {
                let mut state = studio.borrow_mut();
                state.open_menu = -1;
                match action.as_str() {
                    "export" => {
                        state.export.open = true;
                        state.export.phase = ExportPhase::Idle;
                        state.export.message = String::new();
                    }
                    "settings" => state.settings.open = true,
                    "snap" => state.snap = !state.snap,
                    "zoom-in" => {
                        state.seconds_per_pixel = (state.seconds_per_pixel / 1.4).max(0.000_5);
                    }
                    "zoom-out" => {
                        state.seconds_per_pixel = (state.seconds_per_pixel * 1.4).min(1.5);
                    }
                    "start" => {
                        state.playing = false;
                        state.playhead = 0.0;
                    }
                    "end" => {
                        state.playing = false;
                        state.playhead = state.duration();
                    }
                    "delete" => {
                        let doomed = state.selection.clone();
                        state
                            .now_mut()
                            .clips
                            .retain(|clip| !doomed.contains(&clip.id));
                        state.selection.clear();
                    }
                    "split" => {
                        let at = state.playhead;
                        let victims: Vec<String> = state
                            .now()
                            .clips
                            .iter()
                            .filter(|clip| {
                                clip.start + MIN_DURATION < at
                                    && at < clip.end() - MIN_DURATION
                                    && !state.locked(&clip.track)
                            })
                            .map(|clip| clip.id.clone())
                            .collect();
                        for victim in victims {
                            let Some(index) = state.now().clips.iter().position(|c| c.id == victim)
                            else {
                                continue;
                            };
                            let source = state.now().clips[index].clone();
                            let new_id = state.mint("c");
                            let head = at - source.start;
                            let mut tail = source.clone();
                            tail.id = new_id;
                            tail.start = at;
                            tail.duration = source.duration - head;
                            tail.source_start = source.source_start + head;
                            tail.fade_in = 0.0;
                            tail.transition_in = false;
                            state.now_mut().clips[index].duration = head;
                            state.now_mut().clips[index].fade_out = 0.0;
                            state.now_mut().clips.push(tail);
                        }
                    }
                    // Nothing is written: there is no store in this crate.
                    // The notice is the whole of what Save does here, which is
                    // also the shape it keeps — the reference saves and then
                    // says so, and the day this one has a file to write it
                    // reports what happened to it, `failed` and all.
                    "save" => state.notify("Project saved", false),
                    // Templates, speech and closing a project need that same
                    // store. The rows are here so the menu is the shape it
                    // will be; wiring them is a handler each the day it exists.
                    _ => {}
                }
            }
            if action == "close-window" {
                app.window().hide().ok();
                return;
            }
            studio.borrow().publish(&app, &models);
        }
    });

    studio.borrow().publish(&app, &models);

    // --- the pieces Slint cannot express ---------------------------------
    app.global::<Curves>().on_ease(bezier_y_at_x);
    app.global::<Curves>()
        .on_parse(|text, fallback| parse_bezier(text.as_str(), fallback));
    app.global::<Fmt>()
        .on_parse_timecode(|text| parse_timecode(text.as_str()));
    app.global::<Fmt>().on_tick_interval(tick_interval);
    app.global::<Fmt>()
        .on_parse_frames(|text, rate| parse_frames(text.as_str(), rate));

    // The drag payloads. A `data-transfer` is the platform's own drag object,
    // so Slint keeps it opaque and leaves building and reading one to the host
    // language — these two are that language. The payload is plain text
    // because it crosses a boundary that only carries text and images, and
    // because a drag that says "media:12" is one that can be read in a log.
    app.global::<Payload>().on_of(DataTransfer::from);
    app.global::<Payload>()
        .on_text(|payload| payload.plain_text().unwrap_or_default());

    // `pane:2:Inspector:inspector` -> 2. Everything else, including a drag
    // that came from another application, is -1: a seat only accepts a panel,
    // and this is how it tells one from anything else.
    app.global::<Payload>().on_pane_seat(|text| {
        text.strip_prefix("pane:")
            .and_then(|rest| rest.split(':').next())
            .and_then(|seat| seat.parse().ok())
            .unwrap_or(-1)
    });

    // The picture the cursor carries. Resolved through the same `incoming`
    // the drop does, so the chip and the clip it would leave are one answer
    // to one question — a chip that named a different thing than the ghost
    // would be a bug you could only find by looking very hard at a drag.
    //
    // Memoised by payload: the property is read once per source element, and
    // there are as many of those as there are cards in the bin. Rasterising is
    // the renderer's problem and happens once, for the one that is actually in
    // flight; this only builds and parses the document.
    app.global::<Payload>().on_preview({
        let weak = app.as_weak();
        let library = library.clone();
        // Keyed by theme *and* payload: the chip is rasterised in the
        // palette's own colours, so the same clip dragged under the other
        // theme is a different picture. One character in front of the payload
        // is cheaper than reaching in to clear the map when the toggle flips.
        let chips: RefCell<std::collections::HashMap<String, slint::Image>> =
            RefCell::new(std::collections::HashMap::new());
        move |payload| {
            let Some(app) = weak.upgrade() else {
                return slint::Image::default();
            };
            let theme = app.global::<Theme>();
            let key = format!("{}{payload}", if theme.get_dark() { 'd' } else { 'l' });
            if let Some(chip) = chips.borrow().get(&key) {
                return chip.clone();
            }

            // A panel being dragged out of its seat. Not a clip, so it never
            // reaches the library: `pane:<seat>:<name>:<slug>`, and the chip
            // wears the accent rather than a media kind's hue — what is in the
            // air is a piece of the window, not a piece of the edit.
            if let Some(rest) = payload.strip_prefix("pane:") {
                let mut fields = rest.splitn(3, ':').skip(1);
                let label = fields.next().unwrap_or_default();
                let slug = fields.next().unwrap_or_default();
                let chip = slint::Image::load_from_svg_data(
                    drag_chip_svg(
                        pane_glyph(slug),
                        label,
                        "",
                        theme.get_accent(),
                        theme.get_field(),
                        theme.get_raised(),
                        theme.get_fg(),
                    )
                    .as_bytes(),
                )
                .unwrap_or_default();
                chips.borrow_mut().insert(key, chip.clone());
                return chip;
            }

            // An empty payload is a card that opted out of dragging, and an
            // unreadable one is a drag this tree did not start. Both get the
            // empty image, which the overlay skips.
            let Some(plan) = Studio::incoming(payload.as_str(), &library) else {
                return slint::Image::default();
            };
            let (mark, well) = match plan.kind {
                ClipKind::Video => (theme.get_kind_video(), theme.get_kind_video_well()),
                ClipKind::Audio => (theme.get_kind_audio(), theme.get_kind_audio_well()),
                ClipKind::Image => (theme.get_kind_image(), theme.get_kind_image_well()),
                ClipKind::Text => (theme.get_kind_text(), theme.get_kind_text_well()),
                ClipKind::Filter => (theme.get_kind_filter(), theme.get_kind_filter_well()),
            };
            // The envelope the bin card draws, for the one kind that has one.
            let wave = plan
                .media
                .strip_prefix('m')
                .and_then(|id| id.parse().ok())
                .and_then(|id| library.item(id))
                .map(|item| item.wave.to_string())
                .unwrap_or_default();
            let document = drag_chip_svg(
                chip_glyph(plan.kind),
                &plan.label,
                &wave,
                mark,
                well,
                theme.get_raised(),
                theme.get_fg(),
            );
            let chip = slint::Image::load_from_svg_data(document.as_bytes()).unwrap_or_default();
            chips.borrow_mut().insert(key, chip.clone());
            chip
        }
    });

    // --- simulated programme level ---------------------------------------
    //
    // Armed by hover rather than left running for the life of the process.
    //
    // Thirty samples a second is an animation however it is produced: each one
    // writes `level` and `peak`, and the whole window is redrawn for every dirty
    // property — Skia has partial-rendering machinery but it is off on every
    // GPU surface, and FemtoVG has none at all. Left running, the
    // feed alone held the app at ~10% of a core doing nothing but re-drawing a
    // bar nobody was looking at. The meter reports hover (see the TouchArea in
    // level-meter.slint) and the timer follows it, so an unattended window
    // costs no frames at all.
    let sim = Rc::new(LevelSim::new());
    let level_timer = Rc::new(Timer::default());
    app.on_meter_watched_changed({
        let weak = app.as_weak();
        let sim = sim.clone();
        let level_timer = level_timer.clone();
        move |watched| {
            let Some(app) = weak.upgrade() else { return };
            if !watched {
                // Park at silence. Freezing on the last sample would leave the
                // bar stopped mid-signal, which reads as a hung meter; zero
                // reads as no programme, which is what it is. Peak goes
                // negative rather than to zero because that hides the hold
                // marker outright, the same way muting does — a tick pinned at
                // the left edge is just debris.
                level_timer.stop();
                sim.reset();
                app.global::<Editor>().set_level(0.0);
                app.global::<Editor>().set_peak(-1.0);
                return;
            }
            level_timer.start(TimerMode::Repeated, std::time::Duration::from_millis(33), {
                let weak = weak.clone();
                let sim = sim.clone();
                move || {
                    let Some(app) = weak.upgrade() else { return };
                    if app.get_muted() {
                        sim.reset();
                        app.global::<Editor>().set_level(0.0);
                        app.global::<Editor>().set_peak(0.0);
                        return;
                    }
                    let (level, peak) = sim.tick(0.033);
                    app.global::<Editor>().set_level(level);
                    app.global::<Editor>().set_peak(peak);
                }
            });
        }
    });

    app.run()
}

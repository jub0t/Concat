// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Rendering the session to a file.
//!
//! The rendering is `concat-export`'s; what is here is the seam the window
//! uses: the session flattens itself, the window adds the destination and
//! the quality, and the one export that can run at a time reports through a
//! callback and stops through a flag.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use concat_export::{ExportClip, ExportRequest, Reporter, render_audio, render_on};
pub use concat_media::audio::AudioFormat;
pub use concat_media::{ColorRange, RateMode, VideoCodec};
pub use concat_render::WgpuCompositor;

use concat_project::model::Project;

use crate::jobs::{Job, SingleFlight};
use crate::session::Session;

/// What the window decides about an export: where it goes, how good it is,
/// and any rasterised titles rejoining as image clips.
#[derive(Clone, Debug, Default)]
pub struct ExportSpec {
    /// The file to write.
    pub output: String,
    /// Constant rate factor; lower is better quality and a bigger file.
    pub crf: u8,
    /// The x264 speed/size preset name, e.g. "medium".
    pub preset: String,
    /// What to encode to.
    pub codec: VideoCodec,
    /// Ten bits a channel rather than eight.
    pub ten_bit: bool,
    /// VBR (the CRF carries the quality) or CBR (the bitrate is the
    /// target). VBR by default, so the sheet's Advanced section being off
    /// means the file is what it always was.
    pub rate_mode: concat_media::RateMode,
    /// Target bitrate in kbps, used when `rate_mode` is CBR.
    pub bitrate_kbps: u32,
    /// The levels the file is written in and tagged with. Video range,
    /// 16-235, unless the sheet's Advanced section says full.
    /// https://github.com/jub0t/Concat/issues/103
    pub color_range: ColorRange,
    /// An HDR timeline written as HDR, its own HLG or PQ, rather than
    /// tone-mapped to SDR. Nothing for an SDR timeline.
    pub hdr: bool,
}

/// One progress report: which frame of how many, in which stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Frames done.
    pub frame: i64,
    /// Frames in total.
    pub total: i64,
    /// What the exporter is doing: "video", "audio", "mux".
    pub stage: &'static str,
}

/// The session flattened into a request the engine can render.
pub fn request(session: &Session, spec: &ExportSpec, titles: Vec<ExportClip>) -> ExportRequest {
    let settings = session.settings();
    let mut clips = session.flattened_clips();
    clips.extend(titles);
    ExportRequest {
        output: spec.output.clone(),
        width: settings.width,
        height: settings.height,
        rate_num: settings.rate_num,
        rate_den: settings.rate_den,
        crf: spec.crf,
        preset: spec.preset.clone(),
        codec: spec.codec,
        ten_bit: spec.ten_bit,
        rate_mode: spec.rate_mode,
        bitrate_kbps: spec.bitrate_kbps,
        color_range: spec.color_range,
        color_space: session.video().color_space,
        hdr: spec.hdr,
        effect_time_offset: 0.0,
        clips,
    }
}

/// Some clips exported alone: Export Selected.
///
/// Rendered as Merge selected clips renders a selection (see
/// [`crate::consolidate`]): the project with only them on its active
/// timeline, moved so the first starts at zero and the gaps between them
/// kept, every track shown and heard - what was picked is what is wanted
/// - and the effects on the clock they had where the clips were.
#[derive(Clone)]
pub struct Selected {
    /// The project with the selection alone on its active timeline, moved;
    /// what its titles are painted from.
    pub project: Project,
    /// Every clip of it but the titles, flattened as an export takes them.
    pub clips: Vec<ExportClip>,
    /// Where the first of them started on the timeline, in seconds.
    pub origin: f64,
}

impl Selected {
    /// The clips of `project`'s active timeline named in `ids`, alone;
    /// `None` when none of them is there. `folder` is the project's, for
    /// the media its clips name relatively.
    pub fn of(project: &Project, ids: &[String], folder: &Path) -> Option<Self> {
        let origin = project
            .active()
            .clips
            .iter()
            .filter(|clip| ids.contains(&clip.id))
            .map(|clip| clip.start)
            .min_by(f64::total_cmp)?;
        let mut alone = project.clone();
        let timeline = alone.active_mut();
        timeline.clips.retain(|clip| ids.contains(&clip.id));
        for clip in &mut timeline.clips {
            Arc::make_mut(clip).start -= origin;
        }
        for track in &mut timeline.tracks {
            track.visible = true;
            track.muted = false;
        }
        let clips = concat_export::flatten::flatten_timeline_in(&alone, None, Some(folder));
        Some(Self {
            project: alone,
            clips,
            origin,
        })
    }

    /// How long the file runs, in seconds: to the end of the last clip.
    pub fn duration(&self) -> f64 {
        self.project
            .active()
            .clips
            .iter()
            .map(|clip| clip.start + clip.duration)
            .fold(0.0, f64::max)
    }

    /// `request` - made for the whole session by [`request`] - narrowed to
    /// the selection: its clips and `titles`, the selection's own painted
    /// from [`Selected::project`], from zero, the effects on the clock they
    /// had where they were.
    pub fn narrow(&self, request: &mut ExportRequest, titles: Vec<ExportClip>) {
        request.clips = self.clips.clone();
        request.clips.extend(titles);
        request.effect_time_offset = self.origin;
    }
}

/// Renders `request` and returns the path written, reporting through
/// `progress` and stopping at the next frame once `cancel` is set. Blocks for
/// the whole render: run it on its own thread. Draws on a device of its
/// own; [`run_on`] draws on one the caller lends.
pub fn run(
    request: &ExportRequest,
    cancel: &AtomicBool,
    progress: impl FnMut(Progress),
) -> Result<String, String> {
    run_on(request, None, cancel, progress)
}

/// [`run`] drawing on `compositor` where the caller has one to lend: the
/// window's, a sibling of its monitor's (see `Monitor::sibling`), so the
/// export draws on the device that has drawn the monitor all along rather
/// than opening a second one through the drivers, which on Windows laptops
/// with NVIDIA chips took the app down at the first frame (issue #202).
/// `None` draws as [`run`] does.
pub fn run_on(
    request: &ExportRequest,
    compositor: Option<WgpuCompositor>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<String, String> {
    let mut report = |frame: i64, total: i64, stage: &'static str| {
        progress(Progress {
            frame,
            total,
            stage,
        });
    };
    render_on(
        request,
        compositor,
        Reporter {
            progress: &mut report,
            cancel,
        },
    )
}

/// [`run`] for the sound alone: `request` mixed into an audio file, as the
/// format its output's extension names, at `bit_rate` bits a second where
/// the format takes one (`None` for its own), with no picture drawn and
/// so no device needed. Reports and stops as [`run`] does.
pub fn run_audio(
    request: &ExportRequest,
    bit_rate: Option<usize>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<String, String> {
    let mut report = |frame: i64, total: i64, stage: &'static str| {
        progress(Progress {
            frame,
            total,
            stage,
        });
    };
    render_audio(
        request,
        bit_rate,
        Reporter {
            progress: &mut report,
            cancel,
        },
    )
}

/// The slot for the one export that can run at a time - enforced, not
/// documented: a second export while one runs is refused instead of racing
/// the first for its temp files and cancel flag.
#[derive(Clone, Default)]
pub struct Exporter {
    slot: Arc<SingleFlight>,
}

impl Exporter {
    /// An idle exporter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Claims the slot for a new export, whose cancel flag the render polls.
    pub fn begin(&self) -> Result<Job, String> {
        self.slot.begin("export")
    }

    /// Asks the running export to stop at the next frame. Idle is a no-op.
    pub fn cancel(&self) {
        self.slot.cancel();
    }

    /// Whether an export is running.
    pub fn is_busy(&self) -> bool {
        self.slot.is_busy()
    }
}

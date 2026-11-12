// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Rendering the session to a file.
//!
//! The rendering is `concat-export`'s; what is here is the seam the window
//! uses: the session flattens itself, the window adds the destination and
//! the quality, and the one export that can run at a time reports through a
//! callback and stops through a flag.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use concat_export::{ExportClip, ExportRequest, Reporter, render_on};
pub use concat_media::{ColorRange, RateMode, VideoCodec};
pub use concat_render::WgpuCompositor;

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
    /// What the exporter is doing: "rendering", "mixing audio", "muxing";
    /// "rendering in software" ([`concat_export::RENDERING_IN_SOFTWARE`])
    /// when the hardware encoder stopped part way and the video is being
    /// rendered again from its first frame on the CPU, the frame count
    /// back at nothing.
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
        canvas: session.video().canvas,
        clips,
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
    // What a death part way through needs to be read: what was asked for,
    // which stage it was in, and how far that stage had got. The stages
    // and every quarter of one are a handful of lines an export.
    let fps = request.rate_num as f64 / request.rate_den.max(1) as f64;
    log::info!(
        "export: {}x{} {fps:.3} fps · {}{} · {} kbps · {} clips",
        request.width,
        request.height,
        request.codec.name(),
        if request.ten_bit { " 10-bit" } else { "" },
        request.bitrate_kbps,
        request.clips.len(),
    );
    let mut stage_now = "";
    let mut quarter = 0;
    let mut reached = (0, 0);
    let mut report = |frame: i64, total: i64, stage: &'static str| {
        if stage != stage_now {
            log::info!("export: {stage} · {total} frames");
            stage_now = stage;
            quarter = 0;
        }
        let now = if total > 0 { frame * 4 / total } else { 0 };
        if (1..4).contains(&now) && now > quarter {
            log::info!("export: {stage} · frame {frame} of {total}");
            quarter = now;
        }
        reached = (frame, total);
        progress(Progress {
            frame,
            total,
            stage,
        });
    };
    let result = render_on(
        request,
        compositor,
        Reporter {
            progress: &mut report,
            cancel,
        },
    );
    // How it ended, and where its time went, is the engine's own line
    // (`render_on`); this adds only how far a stopped one had got.
    if result.is_err() {
        let (frame, total) = reached;
        log::info!("export: stopped in {stage_now} at frame {frame} of {total}");
    }
    result
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

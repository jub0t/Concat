// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! An export in a process of its own: the [`concat_worker::Job`] the window
//! hands every export to on a desktop.
//!
//! An export runs the most code Concat does not own: the graphics driver
//! under every frame it draws, FFmpeg's decoders and the chip's encoder. In
//! the window's process, any of them crashing took the editor with it, and
//! Direct3D 12 gives a process one device per adapter, so the export losing
//! its device was the preview losing its own (#824, #868). In a worker it
//! has a device of its own; a worker that dies costs the export one try,
//! drawn again on the CPU ([`concat_worker::Policy::default`]), and the
//! window goes on answering throughout.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use concat_core::Halt;
use concat_worker::{Context, Event, Policy};
use serde::{Deserialize, Serialize};

use crate::export::{ColorRange, ExportSpec, Progress, RateMode, VideoCodec};

/// The export job, as the worker table names it.
pub struct Export;

/// Everything an export needs from the window.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Order {
    /// A copy of the edit as it is now, saved or not.
    pub project: concat_project::model::Project,
    /// The project's folder, which its media's paths are relative to.
    pub project_dir: PathBuf,
    /// The file to write.
    pub output: String,
    /// See [`ExportSpec`].
    pub crf: u8,
    /// See [`ExportSpec`].
    pub preset: String,
    /// [`VideoCodec::name`].
    pub codec: String,
    /// See [`ExportSpec`].
    pub ten_bit: bool,
    /// [`RateMode::name`].
    pub rate_mode: String,
    /// See [`ExportSpec`].
    pub bitrate_kbps: u32,
    /// [`ColorRange::name`].
    pub color_range: String,
    /// See [`ExportSpec`].
    pub hdr: bool,
    /// The export's frame.
    pub width: u32,
    /// The export's frame.
    pub height: u32,
    /// The export's rate.
    pub rate_num: i64,
    /// The export's rate.
    pub rate_den: i64,
    /// What the window's encoder probe found: each encoder, and whether it
    /// opened. The worker opens none the probe found does not.
    #[serde(default)]
    pub encoders: Vec<(String, bool)>,
}

/// An export's progress on the wire: [`Progress`] with its stage as text.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Step {
    /// Frames done.
    pub frame: i64,
    /// Frames in all.
    pub total: i64,
    /// [`Progress::stage`].
    pub stage: String,
}

impl Step {
    /// The progress as the window knows it: a stage it has no words for is
    /// "rendering".
    fn progress(&self) -> Progress {
        Progress {
            frame: self.frame,
            total: self.total,
            stage: STAGES
                .into_iter()
                .find(|known| *known == self.stage)
                .unwrap_or("rendering"),
        }
    }
}

/// Every stage the window says in its own words.
const STAGES: [&str; 4] = [
    "rendering",
    concat_export::RENDERING_IN_SOFTWARE,
    "mixing audio",
    "muxing",
];

impl Order {
    /// The order for an export the window would run as `spec` on a copy of
    /// `project`, at its own frame and rate.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        project: concat_project::model::Project,
        project_dir: PathBuf,
        spec: &ExportSpec,
        width: u32,
        height: u32,
        rate_num: i64,
        rate_den: i64,
    ) -> Order {
        Order {
            project,
            project_dir,
            output: spec.output.clone(),
            crf: spec.crf,
            preset: spec.preset.clone(),
            codec: spec.codec.name().to_owned(),
            ten_bit: spec.ten_bit,
            rate_mode: spec.rate_mode.name().to_owned(),
            bitrate_kbps: spec.bitrate_kbps,
            color_range: spec.color_range.name().to_owned(),
            hdr: spec.hdr,
            width,
            height,
            rate_num,
            rate_den,
            encoders: concat_media::encode::known_encoders(),
        }
    }

    /// The spec the window started from, read back.
    fn spec(&self) -> Result<ExportSpec, String> {
        Ok(ExportSpec {
            output: self.output.clone(),
            crf: self.crf,
            preset: self.preset.clone(),
            codec: VideoCodec::parse(&self.codec)
                .ok_or_else(|| format!("unknown codec {:?}", self.codec))?,
            ten_bit: self.ten_bit,
            rate_mode: RateMode::ALL
                .into_iter()
                .find(|mode| mode.name() == self.rate_mode)
                .ok_or_else(|| format!("unknown rate mode {:?}", self.rate_mode))?,
            bitrate_kbps: self.bitrate_kbps,
            color_range: ColorRange::parse(&self.color_range)
                .ok_or_else(|| format!("unknown colour range {:?}", self.color_range))?,
            hdr: self.hdr,
        })
    }
}

impl concat_worker::Job for Export {
    const NAME: &'static str = "export";
    type Request = Order;
    type Progress = Step;
    type Output = String;

    /// The export as the window would run it in its own process: the
    /// titles painted at the export's frame, the edit flattened, and the
    /// render on a device this process opens.
    fn run(order: Order, context: &Context<Step>) -> Result<String, Halt> {
        // For the smoke run that proves a worker's death costs one try and
        // not the export: the first worker dies as a driver's crash would
        // kill it, and the second, drawing on the CPU, is let live.
        if std::env::var_os("CONCAT_TEST_EXPORT_WORKER_ABORT").is_some()
            && !std::env::var("CONCAT_GPU").is_ok_and(|gpu| gpu == "software")
        {
            log::warn!("export: CONCAT_TEST_EXPORT_WORKER_ABORT is set; this worker aborts");
            std::process::abort();
        }
        for (encoder, opened) in &order.encoders {
            concat_media::encode::assume_encoder(encoder, *opened);
        }
        let spec = order.spec().map_err(Halt::Failed)?;
        let dirs = crate::dirs::AppDirs::locate().map_err(Halt::Failed)?;
        let titles = crate::titles::Titles::new(&dirs);
        let cancel = context.cancel_flag();
        let Some(titles) = titles.clips_unless(&order.project, order.width, order.height, cancel)
        else {
            return Err(Halt::Cancelled);
        };
        let titles = titles.into_iter().map(|title| title.clip).collect();
        let mut request =
            crate::export::request_for(&order.project, &order.project_dir, &spec, titles);
        request.width = order.width;
        request.height = order.height;
        request.rate_num = order.rate_num;
        request.rate_den = order.rate_den;
        crate::export::run_on(&request, None, cancel, |progress| {
            context.progress(&Step {
                frame: progress.frame,
                total: progress.total,
                stage: progress.stage.to_owned(),
            });
        })
    }

    /// Every frame drawn says so; the sound's mix and the mux say so only
    /// as they begin, and on a long film and a slow CPU take minutes.
    fn quiet(last: Option<&Step>) -> Duration {
        match last.map(|step| step.stage.as_str()) {
            Some("mixing audio" | "muxing") => Duration::from_secs(30 * 60),
            _ => Duration::from_secs(5 * 60),
        }
    }
}

/// The window's side: runs `order` in a worker of `program`, then once
/// more drawing on the CPU if that worker died, and hands back how it
/// went. `None` when no worker could be started at all, and the window
/// renders in its own process, as before.
pub fn run(
    program: &Path,
    order: &Order,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Option<Result<String, Halt>> {
    concat_worker::run::<Export>(program, order, &Policy::default(), cancel, |event| {
        match event {
            Event::Progress(step) => progress(step.progress()),
            // Said as the in-process export says its own fall to the CPU.
            Event::Retrying { .. } => progress(Progress {
                frame: 0,
                total: 0,
                stage: concat_export::RENDERING_IN_SOFTWARE,
            }),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_reads_back_as_the_window_knows_it() {
        let step = Step {
            frame: 120,
            total: 900,
            stage: "mixing audio".into(),
        };
        assert_eq!(step.progress().stage, "mixing audio");
        let other = Step {
            stage: "tuning the flux".into(),
            ..step
        };
        assert_eq!(other.progress().stage, "rendering");
    }

    #[test]
    fn an_order_reads_back_as_the_spec_it_was_made_from() {
        let spec = ExportSpec {
            output: "out.mp4".into(),
            crf: 18,
            preset: "veryfast".into(),
            codec: VideoCodec::Hevc,
            ten_bit: true,
            rate_mode: RateMode::Average,
            bitrate_kbps: 12_000,
            color_range: ColorRange::Full,
            hdr: true,
        };
        let order = Order::new(
            concat_project::model::Project::new(),
            PathBuf::from("project"),
            &spec,
            1920,
            1080,
            30,
            1,
        );
        let line = serde_json::to_string(&order).expect("an order writes");
        assert!(!line.contains('\n'), "one line");
        let back: Order = serde_json::from_str(&line).expect("an order reads");
        let again = back.spec().expect("the spec reads");
        assert_eq!(again.codec, VideoCodec::Hevc);
        assert_eq!(again.rate_mode, RateMode::Average);
        assert_eq!(again.color_range, ColorRange::Full);
        assert_eq!(
            (again.crf, again.bitrate_kbps, again.hdr),
            (18, 12_000, true)
        );
        assert_eq!(
            (back.width, back.height, back.rate_num, back.rate_den),
            (1920, 1080, 30, 1)
        );
    }

    #[test]
    fn mixing_and_muxing_may_be_quiet_for_longer() {
        use concat_worker::Job as _;
        let at = |stage: &str| Step {
            frame: 0,
            total: 1,
            stage: stage.into(),
        };
        assert!(Export::quiet(Some(&at("muxing"))) > Export::quiet(Some(&at("rendering"))));
        assert_eq!(Export::quiet(None), Export::quiet(Some(&at("rendering"))));
    }
}

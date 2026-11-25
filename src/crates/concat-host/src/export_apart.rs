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
//! has a device of its own; a worker that dies in the graphics driver, or
//! hangs, costs the export one try, drawn again on the CPU
//! ([`concat_worker::Policy::default`]); one that dies anywhere else fails
//! the export there and then, saying how (#762). The window goes on
//! answering throughout.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
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
    /// The ids of the imported packages the window's catalogue holds -
    /// the ones it loaded and whose shaders passed its trial. The worker
    /// loads the same folder and keeps these alone: a worker that loaded
    /// none exported the edit without every imported effect and look the
    /// preview showed, and said nothing.
    pub packages: Vec<String>,
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
            packages: concat_effects::Catalogue::builtin()
                .packages()
                .filter(|package| package.folder.is_some())
                .map(|package| package.id().to_owned())
                .collect(),
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
        // For the runs that prove how a worker's death is taken. Set to
        // `elsewhere`, every worker dies as a crash outside the driver
        // does - the heap's own check, an abort - with no crash line, and
        // the export fails at once, its caller told (#762). Set to anything
        // else, the first worker dies as a crash in the driver kills it -
        // the crash handler's line naming the driver's module, then the
        // abort - and the second, drawing on the CPU, is let live.
        let abort = std::env::var("CONCAT_TEST_EXPORT_WORKER_ABORT").ok();
        if abort.as_deref() == Some("elsewhere") {
            log::warn!("export: CONCAT_TEST_EXPORT_WORKER_ABORT=elsewhere; this worker aborts");
            std::process::abort();
        }
        if abort.is_some() && !std::env::var("CONCAT_GPU").is_ok_and(|gpu| gpu == "software") {
            log::warn!("export: CONCAT_TEST_EXPORT_WORKER_ABORT is set; this worker aborts");
            eprintln!(
                "FATAL [main] crash: 0xC0000005 EXCEPTION_ACCESS_VIOLATION, reading 0x0, \
                 in nvwgf2umx.dll+0x0 (CONCAT_TEST_EXPORT_WORKER_ABORT)"
            );
            std::process::abort();
        }
        for (encoder, opened) in &order.encoders {
            concat_media::encode::assume_encoder(encoder, *opened);
        }
        let dirs = crate::dirs::AppDirs::locate().map_err(Halt::Failed)?;
        install_packages(&dirs.effects(), &order.packages);
        render(
            &order,
            &dirs,
            context.cancel_flag(),
            &mut |progress: Progress| {
                context.progress(&Step {
                    frame: progress.frame,
                    total: progress.total,
                    stage: progress.stage.to_owned(),
                });
            },
        )
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

/// The export `order` asks for, drawn in this process: its titles painted
/// at the export's frame, the edit flattened, and the render on a device
/// this process opens.
fn render(
    order: &Order,
    dirs: &crate::dirs::AppDirs,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<String, Halt> {
    let spec = order.spec().map_err(Halt::Failed)?;
    let titles = crate::titles::Titles::new(dirs);
    let Some(titles) = titles.clips_unless(&order.project, order.width, order.height, cancel)
    else {
        return Err(Halt::Cancelled);
    };
    let titles = titles.into_iter().map(|title| title.clip).collect();
    let mut request = crate::export::request_for(&order.project, &order.project_dir, &spec, titles);
    request.width = order.width;
    request.height = order.height;
    request.rate_num = order.rate_num;
    request.rate_den = order.rate_den;
    crate::export::run_on(&request, None, cancel, progress)
}

/// Loads the imported packages in `dir` that the window holds, `kept`,
/// and leaves out the rest: one the window refused - a shader that failed
/// its trial there - is not drawn here either.
fn install_packages(dir: &Path, kept: &[String]) {
    if kept.is_empty() {
        return;
    }
    let errors = concat_effects::Catalogue::install_with(dir, &mut |package| {
        if kept.iter().any(|id| id == package.id()) {
            Ok(())
        } else {
            Err("the window left it out".to_owned())
        }
    });
    let loaded = concat_effects::Catalogue::builtin()
        .packages()
        .filter(|package| package.folder.is_some())
        .count();
    log::info!(
        "export: {loaded} of the window's {} imported packages loaded",
        kept.len()
    );
    for error in errors {
        log::warn!("export: an imported package was not loaded: {error}");
    }
}

/// The program this process runs export workers of, once its `main` has
/// said it serves them: see [`serve_from`].
static PROGRAM: OnceLock<PathBuf> = OnceLock::new();

/// Says that `program` - this process's own executable - serves [`Export`]
/// workers, so every export started here, the API's as well as the
/// window's, runs in one and a crash in it costs the export, not the
/// process: before, an API export that crashed took the CLI or the window
/// with it, and its caller heard no `export.failed`, only the process
/// gone (#762). A program that never says so - a test binary - exports in
/// its own process.
pub fn serve_from(program: PathBuf) {
    let _ = PROGRAM.set(program);
}

/// The program [`serve_from`] named, unless `CONCAT_EXPORT_IN_PROCESS` is
/// set, which keeps every export in this process for a debugger.
pub fn program() -> Option<&'static Path> {
    if std::env::var_os("CONCAT_EXPORT_IN_PROCESS").is_some() {
        return None;
    }
    PROGRAM.get().map(PathBuf::as_path)
}

/// `order` drawn in this process, on the calling thread: for where no
/// worker could be started. The process's own encoders and packages are
/// already what the order names.
pub fn run_here(
    order: &Order,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<String, Halt> {
    let dirs = crate::dirs::AppDirs::locate().map_err(Halt::Failed)?;
    render(order, &dirs, cancel, progress)
}

/// The window's side: runs `order` in a worker of `program`, then once
/// more drawing on the CPU if that worker died in the graphics driver, and
/// hands back how it went. `None` when no worker could be started at all, and the window
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

    /// A package folder in `dir` named `id`: the built-in Mono, renamed.
    fn imported(dir: &Path, id: &str) {
        let from =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../concat-effects/packages/concat.mono");
        let to = dir.join(id);
        std::fs::create_dir_all(&to).expect("package folder");
        for name in ["effect.toml", "effect.wgsl", "fixtures.toml"] {
            let text = std::fs::read_to_string(from.join(name)).expect("built-in Mono");
            std::fs::write(to.join(name), text.replace("concat.mono", id)).expect("written");
        }
    }

    /// The worker draws what the window holds: the imported packages it
    /// kept, beside the built-ins, and not one it left out.
    #[test]
    fn the_worker_loads_the_imported_packages_the_window_kept() {
        let dir = tempfile::tempdir().expect("temp dir");
        imported(dir.path(), "user.kept");
        imported(dir.path(), "user.refused");
        install_packages(dir.path(), &["user.kept".to_owned()]);
        let catalogue = concat_effects::Catalogue::builtin();
        let has = |id: &str| catalogue.packages().any(|package| package.id() == id);
        assert!(has("user.kept"), "the kept package is drawn");
        assert!(!has("user.refused"), "the one the window left out is not");
        assert!(has("concat.mono"), "the built-ins stay");
    }

    /// An order carries the ids of the imported packages the window holds,
    /// and no built-in's.
    #[test]
    fn an_order_names_only_imported_packages() {
        let spec = ExportSpec {
            output: "out.mp4".into(),
            crf: 18,
            preset: "veryfast".into(),
            codec: VideoCodec::H264,
            ten_bit: false,
            rate_mode: RateMode::ALL[0],
            bitrate_kbps: 0,
            color_range: ColorRange::parse("limited").expect("a range"),
            hdr: false,
        };
        let order = Order::new(
            concat_project::model::Project::default(),
            PathBuf::from("."),
            &spec,
            1920,
            1080,
            30,
            1,
        );
        assert!(
            order.packages.iter().all(|id| !id.starts_with("concat.")),
            "built-ins are the worker's own: {:?}",
            order.packages
        );
    }

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

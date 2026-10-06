// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The paused monitor's true frame.
//!
//! One reader pool for the app's lifetime: its whole value is what stays
//! warm between scrubs. The pool locks per reader, so a frame is decoded on
//! whatever thread the caller chose while another thread decodes ahead on a
//! different file - the window debounces and drops stale results, so a slow
//! decode never wedges anything but itself.
//!
//! With the `gpu` feature and a device from [`Monitor::with_gpu`], a frame
//! is drawn on that device and handed back as a texture: decoded pictures
//! go up once, the composite happens where it is shown, and no pixel comes
//! back down. A frame is therefore two calls and not one -
//! [`Monitor::frame_sources`] anywhere, [`Monitor::texture_of`] on the
//! thread that owns the device - because the drawing is not a thing a
//! worker may do; see `texture_of`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use concat_export::ExportClip;
use concat_project::DocumentSettings;
use concat_project::model::ColorSpace;

/// How far ahead of the playhead playback opens the file a cut leads to;
/// see [`Monitor::prefetch`].
const NEXT_CUT: f64 = 1.0;

/// A frame request: the instant and the size, with the clips coming from
/// the session that owns them.
#[derive(Clone, Copy, Debug)]
pub struct FrameSpec {
    /// The timeline instant to composite, in seconds.
    pub time: f64,
    /// Preview frame width in pixels.
    pub width: u32,
    /// Preview frame height in pixels.
    pub height: u32,
    /// The picture is moving - playing - rather than paused or scrubbed:
    /// what tells the scheduler which way to decode ahead.
    pub moving: bool,
    /// Read each file's proxy where it has one - the low stand-in that
    /// keeps playback smooth - rather than the file. A moving picture at
    /// less than full quality; at Full the file itself plays, since a
    /// proxy drawn at full size is a blur, which is what Full was picked
    /// against.
    pub proxy: bool,
    /// What the timeline is output in: an HDR one keeps its clips' light
    /// above white and is rolled off for the SDR screen.
    pub color_space: ColorSpace,
}

/// The reader pool behind the monitor, shareable across threads.
#[derive(Clone)]
pub struct Monitor {
    pool: Arc<concat_media::ReaderPool>,
    /// The last clip list's plan, kept until the list changes: playback
    /// and scrubbing ask for many instants of one document, and the plan
    /// is the half of a frame that does not depend on the instant.
    plan: Arc<Mutex<Option<PlanEntry>>>,
    /// The files a coming cut leads into that playback has already sent
    /// to be opened early, so each is opened once and not once a frame.
    early: Arc<Mutex<Vec<PathBuf>>>,
    #[cfg(feature = "gpu")]
    gpu: Option<Arc<Mutex<concat_render::WgpuCompositor>>>,
}

/// One kept plan and what it was built for.
struct PlanEntry {
    clips: Arc<Vec<ExportClip>>,
    width: u32,
    height: u32,
    rate: (i64, i64),
    color_space: ColorSpace,
    plan: Arc<concat_export::PreviewPlan>,
}

/// The wgpu the monitor's textures belong to.
#[cfg(feature = "gpu")]
pub use concat_render::wgpu;

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

impl Monitor {
    /// A monitor with the engine's default pool budget.
    pub fn new() -> Self {
        Self {
            pool: Arc::clone(crate::scheduler().pool()),
            plan: Arc::new(Mutex::new(None)),
            early: Arc::new(Mutex::new(Vec::new())),
            #[cfg(feature = "gpu")]
            gpu: None,
        }
    }

    /// A monitor that composites on `device` - the window's - so
    /// [`Monitor::texture_of`] yields textures the window shows as they
    /// are.
    #[cfg(feature = "gpu")]
    pub fn with_gpu(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            pool: Arc::clone(crate::scheduler().pool()),
            plan: Arc::new(Mutex::new(None)),
            early: Arc::new(Mutex::new(Vec::new())),
            gpu: Some(Arc::new(Mutex::new(
                concat_render::WgpuCompositor::with_device(device, queue),
            ))),
        }
    }

    /// Lets the GPU's frame textures go down to a quarter of their budget,
    /// for when playback stops: the pool fills as frames stream past, and a
    /// still picture needs only the few it shows. Nothing without a GPU.
    pub fn shrink(&self) {
        #[cfg(feature = "gpu")]
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .shrink_pool(0.25);
        }
    }

    /// A compositor of its own on the window's device, for work off the
    /// monitor's thread - the effect cards. None without a GPU here.
    #[cfg(feature = "gpu")]
    pub fn sibling(&self) -> Option<concat_render::WgpuCompositor> {
        let gpu = self.gpu.as_ref()?;
        let gpu = gpu.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Some(gpu.sibling())
    }

    /// Whether frames can be composited on the GPU.
    pub fn has_gpu(&self) -> bool {
        #[cfg(feature = "gpu")]
        {
            self.gpu.is_some()
        }
        #[cfg(not(feature = "gpu"))]
        {
            false
        }
    }

    /// Makes `frame` the still the pool serves under `path`, a name no
    /// file has; see `ReaderPool::hold_still`.
    pub fn hold_still(&self, path: &std::path::Path, frame: Arc<concat_core::frame::Frame>) {
        self.pool.hold_still(path, frame);
    }

    /// The pictures a monitor frame is made of, decoded and placed but not
    /// yet drawn.
    ///
    /// The half of a frame that is safe on any thread: it reads files and
    /// the reader pool and never touches the device. The other half is
    /// [`Monitor::texture_of`], which is safe on exactly one - see there
    /// for why the two are split at all.
    #[cfg(feature = "gpu")]
    pub fn frame_sources(
        &self,
        clips: Arc<Vec<ExportClip>>,
        settings: &DocumentSettings,
        spec: FrameSpec,
    ) -> Result<concat_export::PreviewSources, String> {
        let plan = self.plan_for(clips, settings, spec);
        concat_export::preview_sources_of(&self.pool, &plan, spec.time, spec.proxy)
    }

    /// Draws [`Monitor::frame_sources`] into a texture on the device this
    /// monitor was given: `Rgba8Unorm`, `spec.width` by `spec.height`,
    /// bindable and renderable. Errs without a device, or once the device
    /// is lost.
    ///
    /// # Call this from the thread that owns the device, and nowhere else
    ///
    /// That device is the window's, and the window's renderer took the
    /// native queue out of it and submits to that queue itself, from the
    /// event loop, outside anything wgpu locks. A queue is externally
    /// synchronised in every one of the three APIs underneath: two threads
    /// submitting to one is undefined, and what it does is not a wrong
    /// pixel. On Mesa's Intel driver it corrupts the submission the driver
    /// is building, the GPU hangs on the bad batch, and the reset takes the
    /// device down for every process on the machine - the editor, the
    /// player, the browser - until the machine is restarted (#70).
    ///
    /// So the decode goes to a worker and the drawing comes back here.
    #[cfg(feature = "gpu")]
    pub fn texture_of(
        &self,
        sources: &concat_export::PreviewSources,
        spec: FrameSpec,
    ) -> Result<wgpu::Texture, String> {
        let gpu = self
            .gpu
            .as_ref()
            .ok_or_else(|| "the monitor has no GPU device".to_owned())?;
        let mut gpu = gpu.lock().map_err(|_| "compositor poisoned".to_owned())?;
        if sources.needs_cpu() {
            // A packaged transition, and nothing else in the way, is drawn
            // whole on the device with no round trip through memory.
            if let Some(texture) = sources.transition_texture(&mut gpu) {
                return Ok(texture);
            }
            // A layer that needs FFmpeg for a package with no shader takes
            // the frame through the CPU; the picture then goes up as one
            // layer of its own.
            let frame = sources.composite(&mut *gpu);
            let mut plan = concat_render::FramePlan::empty(spec.width, spec.height);
            plan.layers.push(concat_render::PlannedLayer::picture(
                concat_render::detached_clip(),
                Arc::new(frame),
            ));
            plan.output = sources.plan().output;
            return gpu
                .render_texture(&plan)
                .ok_or_else(|| "the GPU device was lost".to_owned());
        }
        gpu.render_texture(sources.plan())
            .ok_or_else(|| "the GPU device was lost".to_owned())
    }

    /// The plan for this clip list at this size and rate: the kept one
    /// when it was built for the same list - the same allocation, or an
    /// equal one - and a fresh one otherwise, kept in its place.
    fn plan_for(
        &self,
        clips: Arc<Vec<ExportClip>>,
        settings: &DocumentSettings,
        spec: FrameSpec,
    ) -> Arc<concat_export::PreviewPlan> {
        let rate = (settings.rate_num, settings.rate_den);
        let mut slot = self
            .plan
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = slot.as_ref()
            && entry.width == spec.width
            && entry.height == spec.height
            && entry.rate == rate
            && entry.color_space == spec.color_space
            && (Arc::ptr_eq(&entry.clips, &clips) || *entry.clips == *clips)
        {
            return Arc::clone(&entry.plan);
        }
        let plan = Arc::new(concat_export::preview_plan(
            &clips,
            spec.width,
            spec.height,
            rate.0,
            rate.1,
            spec.color_space,
        ));
        *slot = Some(PlanEntry {
            clips,
            width: spec.width,
            height: spec.height,
            rate,
            color_space: spec.color_space,
            plan: Arc::clone(&plan),
        });
        plan
    }

    /// The engine-composited frame at one instant, as raw RGBA bytes:
    /// exactly `width * height * 4` of them.
    pub fn frame(
        &self,
        clips: Arc<Vec<ExportClip>>,
        settings: &DocumentSettings,
        spec: FrameSpec,
    ) -> Result<Vec<u8>, String> {
        let plan = self.plan_for(clips, settings, spec);
        let sources = concat_export::preview_sources_of(&self.pool, &plan, spec.time, spec.proxy)?;
        sources.pixels()
    }

    /// Decode-ahead for the playback stream: hands the scheduler the next
    /// `frames` instants after `spec.time`, so the following
    /// [`Monitor::frame`] pulls are cache hits instead of decode waits.
    /// Clamped, so a confused caller cannot queue a long decode march. The
    /// proxies are read where [`Monitor::frame`] would read them, so what
    /// is decoded ahead is what will be asked for.
    pub fn prefetch(
        &self,
        clips: Arc<Vec<ExportClip>>,
        settings: &DocumentSettings,
        spec: FrameSpec,
        frames: u32,
    ) {
        let plan = self.plan_for(clips, settings, spec);
        let moments = concat_export::preview_moments(
            &plan,
            spec.time,
            frames.min(concat_media::prefetch::AHEAD),
            spec.proxy,
        );
        let fps = (settings.rate_num as f64 / settings.rate_den.max(1) as f64).max(1.0);
        // The file a cut leads into, read a second early: opening a reader
        // and seeking it costs more than the quarter second the frames
        // ahead cover, so a cut would otherwise wait on it. Only files the
        // frames ahead do not read already, and not pinned - this is to
        // have the reader open and its first frames cached by the cut.
        // Once per file: this runs every frame, the pool opens a reader
        // outside its lock, and a file sent again before its first open
        // finished would be opened again beside it.
        let mut early = self
            .early
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !spec.moving {
            early.clear();
        } else {
            let far = concat_export::preview_moments(&plan, spec.time + NEXT_CUT, 1, spec.proxy)
                .pop()
                .map(|far| far.frames)
                .unwrap_or_default();
            let near = |path: &PathBuf| {
                moments
                    .iter()
                    .any(|near| near.frames.iter().any(|read| &read.path == path))
            };
            // A file stays sent while a cut ahead or the frames ahead
            // still read it; after that, a later cut into it opens it anew.
            early.retain(|path| near(path) || far.iter().any(|read| &read.path == path));
            let fresh: Vec<_> = far
                .into_iter()
                .filter(|request| {
                    !request.still && !near(&request.path) && !early.contains(&request.path)
                })
                .collect();
            early.extend(fresh.iter().map(|request| request.path.clone()));
            if !fresh.is_empty() {
                let pool = Arc::clone(&self.pool);
                crate::scheduler().submit(concat_media::Priority::Filmstrip, move || {
                    for request in &fresh {
                        let _ = pool.frame(request);
                    }
                });
            }
        }
        drop(early);
        crate::scheduler().advance(
            concat_media::Cursor {
                time: spec.time,
                direction: concat_media::Direction::Forward,
                rate: if spec.moving { 1.0 } else { 0.0 },
            },
            1.0 / fps,
            moments,
        );
    }

    /// Forgets every cached frame, reader and plan, for when the project
    /// closes.
    pub fn clear(&self) {
        self.pool.clear();
        *self
            .plan
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.early
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

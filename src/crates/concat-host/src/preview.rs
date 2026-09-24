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

use std::sync::{Arc, Mutex};

use concat_export::ExportClip;
use concat_project::DocumentSettings;

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
    /// what reads a file's proxy where it has one, and tells the
    /// scheduler which way to decode ahead.
    pub moving: bool,
}

/// The reader pool behind the monitor, shareable across threads.
#[derive(Clone)]
pub struct Monitor {
    pool: Arc<concat_media::ReaderPool>,
    /// The last clip list's plan, kept until the list changes: playback
    /// and scrubbing ask for many instants of one document, and the plan
    /// is the half of a frame that does not depend on the instant.
    plan: Arc<Mutex<Option<PlanEntry>>>,
    #[cfg(feature = "gpu")]
    gpu: Option<Arc<Mutex<concat_render::WgpuCompositor>>>,
}

/// One kept plan and what it was built for.
struct PlanEntry {
    clips: Arc<Vec<ExportClip>>,
    width: u32,
    height: u32,
    rate: (i64, i64),
    gpu: bool,
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
            gpu: Some(Arc::new(Mutex::new(
                concat_render::WgpuCompositor::with_device(device, queue),
            ))),
        }
    }

    /// Runs `pass` once on the window's device over a picture `side`
    /// pixels square, against `timeout`: what a custom package's shader
    /// survives before the package is offered. None without a GPU, where
    /// there is nothing to run it on.
    #[cfg(feature = "gpu")]
    pub fn trial(
        &self,
        pass: &concat_core::shader::ShaderPass,
        side: u32,
        timeout: std::time::Duration,
    ) -> Option<Result<(), String>> {
        let gpu = self.gpu.as_ref()?;
        let mut gpu = gpu.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Some(gpu.trial_at(pass, side, timeout))
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
        let plan = self.plan_for(clips, settings, spec, true);
        concat_export::preview_sources_of(&self.pool, &plan, spec.time, spec.moving)
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
            // A layer that needs FFmpeg for a package with no shader takes
            // the frame through the CPU; the picture then goes up as one
            // layer of its own.
            let frame = sources.composite(&mut *gpu);
            let mut plan = concat_render::FramePlan::empty(spec.width, spec.height);
            plan.layers.push(concat_render::PlannedLayer::picture(
                concat_render::detached_clip(),
                Arc::new(frame),
            ));
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
        gpu: bool,
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
            && entry.gpu == gpu
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
            gpu,
        ));
        *slot = Some(PlanEntry {
            clips,
            width: spec.width,
            height: spec.height,
            rate,
            gpu,
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
        let plan = self.plan_for(clips, settings, spec, false);
        let sources = concat_export::preview_sources_of(&self.pool, &plan, spec.time, spec.moving)?;
        Ok(sources
            .composite(&mut concat_render::CpuCompositor)
            .into_pixels())
    }

    /// Decode-ahead for the playback stream: hands the scheduler the next
    /// `frames` instants after `spec.time`, so the following
    /// [`Monitor::frame`] pulls are cache hits instead of decode waits.
    /// Clamped, so a confused caller cannot queue a long decode march. A
    /// moving picture reads proxies, the way [`Monitor::frame`] does for
    /// it, so what is decoded ahead is what will be asked for.
    pub fn prefetch(
        &self,
        clips: Arc<Vec<ExportClip>>,
        settings: &DocumentSettings,
        spec: FrameSpec,
        frames: u32,
    ) {
        let plan = self.plan_for(clips, settings, spec, self.has_gpu());
        let moments = concat_export::preview_moments(&plan, spec.time, frames.min(8), spec.moving);
        let fps = (settings.rate_num as f64 / settings.rate_den.max(1) as f64).max(1.0);
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
    }
}

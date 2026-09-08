// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The paused monitor's true frame.
//!
//! One reader pool for the app's lifetime: its whole value is what stays
//! warm between scrubs. The pool locks per reader, so a frame is composited
//! on whatever thread the caller chose while another thread decodes ahead
//! on a different file - the window debounces and drops stale results, so a
//! slow decode never wedges anything but itself.
//!
//! With the `gpu` feature and a device from [`Monitor::with_gpu`], a frame
//! is composited on that device and handed back as a texture: decoded
//! pictures go up once, the composite happens where it is shown, and no
//! pixel comes back down.

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
            pool: Arc::new(concat_media::ReaderPool::with_defaults()),
            plan: Arc::new(Mutex::new(None)),
            #[cfg(feature = "gpu")]
            gpu: None,
        }
    }

    /// A monitor that composites on `device` - the window's - so
    /// [`Monitor::frame_texture`] yields textures the window shows as they
    /// are.
    #[cfg(feature = "gpu")]
    pub fn with_gpu(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            pool: Arc::new(concat_media::ReaderPool::with_defaults()),
            plan: Arc::new(Mutex::new(None)),
            gpu: Some(Arc::new(Mutex::new(
                concat_render::WgpuCompositor::with_device(device, queue),
            ))),
        }
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

    /// The engine-composited frame at one instant, as a texture on the
    /// device this monitor was given: `Rgba8Unorm`, `spec.width` by
    /// `spec.height`, bindable and renderable. Errs without a device, or
    /// once the device is lost.
    #[cfg(feature = "gpu")]
    pub fn frame_texture(
        &self,
        clips: Arc<Vec<ExportClip>>,
        settings: &DocumentSettings,
        spec: FrameSpec,
    ) -> Result<wgpu::Texture, String> {
        let gpu = self
            .gpu
            .as_ref()
            .ok_or_else(|| "the monitor has no GPU device".to_owned())?;
        let plan = self.plan_for(clips, settings, spec, true);
        let sources = concat_export::preview_sources_of(&self.pool, &plan, spec.time)?;
        let mut gpu = gpu.lock().map_err(|_| "compositor poisoned".to_owned())?;
        if sources.has_treatments() {
            // A layer whose look is a shader is applied where the stack is
            // drawn, on the GPU; only a layer that needs FFmpeg for a
            // package with no shader takes the frame through the CPU.
            if let Some(treatments) = sources.live_treatments() {
                let layers = sources.placed();
                return gpu
                    .composite_texture_treated(
                        spec.width,
                        spec.height,
                        sources.seconds(),
                        &layers,
                        &treatments,
                    )
                    .ok_or_else(|| "the GPU device was lost".to_owned());
            }
            let frame = sources.composite(&mut *gpu);
            let layers = [concat_render::Layer::new(&frame)];
            return gpu
                .composite_texture(spec.width, spec.height, &layers)
                .ok_or_else(|| "the GPU device was lost".to_owned());
        }
        let layers = sources.layers();
        gpu.composite_texture(spec.width, spec.height, &layers)
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
        let mut slot = self.plan.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
        let sources = concat_export::preview_sources_of(&self.pool, &plan, spec.time)?;
        Ok(sources
            .composite(&mut concat_render::CpuCompositor)
            .into_pixels())
    }

    /// Decode-ahead for the playback stream: warms the pool for the next
    /// `frames` instants after `spec.time`, so the following [`Monitor::frame`]
    /// pulls are cache hits instead of decode waits. Clamped, so a confused
    /// caller cannot park the pool's mutex on a long decode march.
    pub fn prefetch(
        &self,
        clips: Arc<Vec<ExportClip>>,
        settings: &DocumentSettings,
        spec: FrameSpec,
        frames: u32,
    ) {
        let plan = self.plan_for(clips, settings, spec, self.has_gpu());
        concat_export::preview_prefetch_of(&self.pool, &plan, spec.time, frames.min(8));
    }

    /// Forgets every cached frame, reader and plan, for when the project
    /// closes.
    pub fn clear(&self) {
        self.pool.clear();
        *self.plan.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

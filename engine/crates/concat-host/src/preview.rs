// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The paused monitor's true frame.
//!
//! One reader pool for the app's lifetime: its whole value is what stays
//! warm between scrubs. Frames are composited through the pool's mutex, one
//! at a time, in order, on whatever thread the caller chose - the window
//! debounces and drops stale results, so a slow decode never wedges anything
//! but itself.
//!
//! With the `gpu` feature and a device from [`Monitor::with_gpu`], a frame
//! is composited on that device and handed back as a texture: decoded
//! pictures go up once, the composite happens where it is shown, and no
//! pixel comes back down.

use std::sync::{Arc, Mutex};

use concat_export::{ExportClip, PreviewFrameRequest};
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
    pool: Arc<Mutex<concat_media::ReaderPool>>,
    #[cfg(feature = "gpu")]
    gpu: Option<Arc<Mutex<concat_render::WgpuCompositor>>>,
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
            pool: Arc::new(Mutex::new(concat_media::ReaderPool::with_defaults())),
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
            pool: Arc::new(Mutex::new(concat_media::ReaderPool::with_defaults())),
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
        clips: Vec<ExportClip>,
        settings: &DocumentSettings,
        spec: FrameSpec,
    ) -> Result<wgpu::Texture, String> {
        let gpu = self
            .gpu
            .as_ref()
            .ok_or_else(|| "the monitor has no GPU device".to_owned())?;
        let request = Self::request(clips, settings, spec);
        let mut pool = self
            .pool
            .lock()
            .map_err(|_| "reader pool poisoned".to_owned())?;
        let sources = concat_export::preview_sources(&mut pool, &request)?;
        let layers = sources.layers();
        let mut gpu = gpu.lock().map_err(|_| "compositor poisoned".to_owned())?;
        gpu.composite_texture(spec.width, spec.height, &layers)
            .ok_or_else(|| "the GPU device was lost".to_owned())
    }

    fn request(
        clips: Vec<ExportClip>,
        settings: &DocumentSettings,
        spec: FrameSpec,
    ) -> PreviewFrameRequest {
        PreviewFrameRequest {
            time: spec.time,
            width: spec.width,
            height: spec.height,
            rate_num: settings.rate_num,
            rate_den: settings.rate_den,
            clips,
        }
    }

    /// The engine-composited frame at one instant, as raw RGBA bytes:
    /// exactly `width * height * 4` of them.
    pub fn frame(
        &self,
        clips: Vec<ExportClip>,
        settings: &DocumentSettings,
        spec: FrameSpec,
    ) -> Result<Vec<u8>, String> {
        let request = Self::request(clips, settings, spec);
        let mut pool = self
            .pool
            .lock()
            .map_err(|_| "reader pool poisoned".to_owned())?;
        concat_export::preview_frame(&mut pool, &request)
    }

    /// Decode-ahead for the playback stream: warms the pool for the next
    /// `frames` instants after `spec.time`, so the following [`Monitor::frame`]
    /// pulls are cache hits instead of decode waits. Clamped, so a confused
    /// caller cannot park the pool's mutex on a long decode march.
    pub fn prefetch(
        &self,
        clips: Vec<ExportClip>,
        settings: &DocumentSettings,
        spec: FrameSpec,
        frames: u32,
    ) {
        let request = Self::request(clips, settings, spec);
        if let Ok(mut pool) = self.pool.lock() {
            concat_export::preview_prefetch(&mut pool, &request, frames.min(8));
        }
    }

    /// Forgets every cached frame and reader, for when the project closes.
    pub fn clear(&self) {
        if let Ok(mut pool) = self.pool.lock() {
            pool.clear();
        }
    }
}

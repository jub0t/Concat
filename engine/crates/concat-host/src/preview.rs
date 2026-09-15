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

use crate::proxy::ProxyStore;

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
    /// Live playback or an active pointer gesture may use a prepared
    /// low-resolution decode copy. Paused truth and export use the source.
    pub live: bool,
    /// Prepare large-video copies in the background before playback starts.
    pub prewarm: bool,
}

/// The reader pool behind the monitor, shareable across threads.
#[derive(Clone)]
pub struct Monitor {
    pool: Arc<concat_media::ReaderPool>,
    /// The last clip list's plan, kept until the list changes: playback
    /// and scrubbing ask for many instants of one document, and the plan
    /// is the half of a frame that does not depend on the instant.
    plan: Arc<Mutex<Option<PlanEntry>>>,
    proxies: ProxyStore,
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
            proxies: ProxyStore::default(),
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
            pool: Arc::new(concat_media::ReaderPool::with_defaults()),
            plan: Arc::new(Mutex::new(None)),
            proxies: ProxyStore::default(),
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
        concat_export::preview_sources_of(&self.pool, &plan, spec.time)
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
        let clips = if spec.live || spec.prewarm {
            self.proxies.clips(clips, spec.live, spec.time)
        } else {
            clips
        };
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
        self.proxies.clear();
        *self
            .plan
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_project::model::{ClipMask, KeyEase, MaskKey, MaskProperty, MaskShape};

    /// Optional local-media diagnostic: live playback may decode a proxy,
    /// but a settled preview must return to the original path.
    #[test]
    #[ignore]
    fn real_media_live_mask_preview_uses_proxy_and_settled_uses_source() {
        let source = std::path::PathBuf::from(std::env::var_os("CONCAT_DIAG_MEDIA").unwrap());
        let video = concat_media::probe(&source).unwrap().video.unwrap();
        let source_at = std::env::var("CONCAT_DIAG_AT")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0);
        let mut clip: ExportClip = serde_json::from_value(serde_json::json!({
            "path": source.to_string_lossy(),
            "kind": "video",
            "start": 0.0,
            "duration": 1.0,
            "sourceStart": source_at,
            "track": 0,
            "hidden": false,
            "muted": true,
            "mediaWidth": video.width,
            "mediaHeight": video.height
        }))
        .unwrap();
        let mut mask = ClipMask::new("mask1".to_owned(), MaskShape::Heart);
        mask.keys = vec![
            MaskKey {
                property: MaskProperty::PositionX,
                at: 0.0,
                value: -0.3,
                ease: KeyEase::LINEAR,
            },
            MaskKey {
                property: MaskProperty::PositionX,
                at: 1.0,
                value: 0.3,
                ease: KeyEase::LINEAR,
            },
        ];
        clip.masks_enabled = true;
        clip.masks.push(mask);
        clip.animation = [1.0, 0.12]
            .into_iter()
            .enumerate()
            .map(|(index, value)| concat_export::ExportKey {
                property: "scale".to_owned(),
                at: index as f64,
                value,
                ease: [0.0, 0.0, 1.0, 1.0],
            })
            .collect();
        let clips = Arc::new(vec![clip]);
        let settings = DocumentSettings {
            name: "diagnostic".to_owned(),
            width: 480,
            height: 270,
            rate_num: 30,
            rate_den: 1,
        };
        let monitor = Monitor::new();
        let start = std::time::Instant::now();
        loop {
            let resolved = monitor.proxies.clips(Arc::clone(&clips), true, 0.0);
            if resolved[0].path != clips[0].path {
                break;
            }
            assert!(start.elapsed().as_secs() < 60, "proxy did not become ready");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let mut samples = Vec::new();
        for index in 0..30 {
            let start = std::time::Instant::now();
            let bytes = monitor
                .frame(
                    Arc::clone(&clips),
                    &settings,
                    FrameSpec {
                        time: index as f64 / 30.0,
                        width: 480,
                        height: 270,
                        live: true,
                        prewarm: true,
                    },
                )
                .unwrap();
            assert_eq!(bytes.len(), 480 * 270 * 4);
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "480×270 live animated scale-and-mask: median {:.2} ms, p90 {:.2} ms",
            samples[15], samples[27]
        );
        assert_ne!(
            monitor.plan.lock().unwrap().as_ref().unwrap().clips[0].path,
            clips[0].path
        );
        monitor
            .frame(
                Arc::clone(&clips),
                &DocumentSettings {
                    width: 960,
                    height: 540,
                    ..settings
                },
                FrameSpec {
                    time: 0.5,
                    width: 960,
                    height: 540,
                    live: false,
                    prewarm: false,
                },
            )
            .unwrap();
        assert_eq!(
            monitor.plan.lock().unwrap().as_ref().unwrap().clips[0].path,
            clips[0].path
        );
    }
}

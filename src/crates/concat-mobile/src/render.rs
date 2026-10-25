// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The thread that owns the preview Surface and the GPU device behind it.
//!
//! The host's rule is that a monitor frame is decoded anywhere and drawn
//! only on the thread that owns the device (`Monitor::texture_of`). With no
//! Slint renderer here, this thread is that owner: it makes the wgpu device
//! on the Surface's ANativeWindow, hands it to a `Monitor::with_gpu`, and
//! draws each frame's texture straight onto the Surface - no readback, which
//! the Slint phone build cannot avoid today (its monitor composites on the
//! CPU, platform.rs). The CPU path (`Monitor::frame` into a locked
//! ANativeWindow buffer) is kept beside it for comparison.
//!
//! Paused, it sleeps until a seek arrives and draws only the newest one.
//! Playing, it draws at the timeline's rate from the audio clock, skipping a
//! frame it is late for rather than queueing it.
// Off Android there is no Surface, so the drawing half is never reached.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use concat_export::ExportClip;
use concat_host::playback::Playback;
use concat_project::DocumentSettings;
use concat_project::model::ColorSpace;

use crate::Stats;

enum Msg {
    Attach {
        window: u64,
        width: u32,
        height: u32,
        mode: String,
        reply: mpsc::Sender<Result<String, String>>,
    },
    Detach {
        reply: mpsc::Sender<()>,
    },
    Seek {
        at: f64,
        sent: Instant,
    },
    Edit {
        clips: Arc<Vec<ExportClip>>,
        settings: DocumentSettings,
        color: ColorSpace,
    },
    Playing {
        on: bool,
        duration: f64,
    },
    ResetStats,
}

/// The edit as the monitor draws it.
struct Doc {
    clips: Arc<Vec<ExportClip>>,
    settings: DocumentSettings,
    color: ColorSpace,
}

/// The render thread, as the engine holds it.
pub struct Handle {
    tx: Mutex<mpsc::Sender<Msg>>,
    stats: Arc<Mutex<Stats>>,
}

impl Handle {
    /// Starts the thread.
    pub fn start(playback: Arc<Playback>) -> Handle {
        let (tx, rx) = mpsc::channel();
        let stats = Arc::new(Mutex::new(Stats::default()));
        let shared = Arc::clone(&stats);
        std::thread::Builder::new()
            .name("spike-render".into())
            .spawn(move || run(rx, playback, shared))
            .expect("could not start the render thread");
        Handle {
            tx: Mutex::new(tx),
            stats,
        }
    }

    fn send(&self, msg: Msg) {
        let _ = self
            .tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .send(msg);
    }

    /// Draws on `window` from now on; see `Engine::attach_surface`.
    pub fn attach(
        &self,
        window: u64,
        width: u32,
        height: u32,
        mode: &str,
    ) -> Result<String, String> {
        let (reply, answer) = mpsc::channel();
        self.send(Msg::Attach {
            window,
            width,
            height,
            mode: mode.to_owned(),
            reply,
        });
        answer
            .recv()
            .unwrap_or_else(|_| Err("the render thread is gone".to_owned()))
    }

    /// Stops drawing on the window, and returns once it has.
    pub fn detach(&self) {
        let (reply, answer) = mpsc::channel();
        self.send(Msg::Detach { reply });
        let _ = answer.recv();
    }

    /// Shows the frame at `at`.
    pub fn seek(&self, at: f64) {
        self.send(Msg::Seek {
            at,
            sent: Instant::now(),
        });
    }

    /// A new edit to draw.
    pub fn edit(&self, clips: Arc<Vec<ExportClip>>, settings: DocumentSettings, color: ColorSpace) {
        self.send(Msg::Edit {
            clips,
            settings,
            color,
        });
    }

    /// Playback started or stopped.
    pub fn playing(&self, on: bool, duration: f64) {
        self.send(Msg::Playing { on, duration });
    }

    /// The numbers so far.
    pub fn stats(&self) -> Stats {
        self.stats
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Starts the numbers again.
    pub fn reset_stats(&self) {
        self.send(Msg::ResetStats);
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// The frame size for a surface: the timeline's frame fitted inside it,
/// never larger than the frame itself, even in both directions.
pub(crate) fn fit(video: (u32, u32), surface: (u32, u32)) -> (u32, u32) {
    let (vw, vh) = (video.0.max(2) as f64, video.1.max(2) as f64);
    let (sw, sh) = (surface.0.max(2) as f64, surface.1.max(2) as f64);
    let scale = (sw / vw).min(sh / vh).min(1.0);
    let even = |value: f64| ((value.round() as u32) & !1).max(2);
    (even(vw * scale), even(vh * scale))
}

fn run(rx: mpsc::Receiver<Msg>, playback: Arc<Playback>, stats: Arc<Mutex<Stats>>) {
    let mut target: Option<target::Target> = None;
    let mut doc: Option<Doc> = None;
    let mut playing = false;
    let mut duration = 0.0f64;
    let mut pending: Option<(f64, Instant)> = None;
    let mut next_frame = Instant::now();
    let mut window_start = Instant::now();
    let mut window_frames = 0u32;
    let mut scrub_total = 0.0f64;
    let stat = |with: &dyn Fn(&mut Stats)| {
        with(
            &mut stats
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
    };

    loop {
        let message = if playing && target.is_some() {
            let wait = next_frame.saturating_duration_since(Instant::now());
            match rx.recv_timeout(wait) {
                Ok(message) => Some(message),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(message) => Some(message),
                Err(_) => return,
            }
        };
        let mut messages: Vec<Msg> = message.into_iter().collect();
        messages.extend(rx.try_iter());
        for message in messages {
            match message {
                Msg::Attach {
                    window,
                    width,
                    height,
                    mode,
                    reply,
                } => {
                    // A new window replaces the old: drop the old surface
                    // first, the device stays.
                    if let Some(old) = target.take() {
                        old.release_surface();
                    }
                    let made = target::Target::attach(window, width, height, &mode);
                    let answer = made
                        .as_ref()
                        .map(|made| made.describe())
                        .map_err(Clone::clone);
                    if let Ok(made) = made {
                        stat(&|stats| {
                            *stats = Stats {
                                renderer: made.describe(),
                                ..Stats::default()
                            }
                        });
                        scrub_total = 0.0;
                        target = Some(made);
                    } else if let Err(error) = &answer {
                        log::error!("could not attach the surface: {error}");
                        stat(&|stats| stats.last_error = error.clone());
                    }
                    let _ = reply.send(answer);
                }
                Msg::Detach { reply } => {
                    if let Some(old) = target.take() {
                        old.release_surface();
                    }
                    let _ = reply.send(());
                }
                Msg::Seek { at, sent } => {
                    // The newest seek wins; the time it waited counts from
                    // the first one it superseded, as a finger feels it.
                    let since = pending.map_or(sent, |(_, first)| first);
                    pending = Some((at, since));
                }
                Msg::Edit {
                    clips,
                    settings,
                    color,
                } => {
                    duration = clips
                        .iter()
                        .map(|clip| clip.start + clip.duration)
                        .fold(0.0, f64::max);
                    doc = Some(Doc {
                        clips,
                        settings,
                        color,
                    });
                    if pending.is_none() {
                        pending = Some((playback.position_now(), Instant::now()));
                    }
                }
                Msg::Playing {
                    on,
                    duration: total,
                } => {
                    playing = on;
                    duration = total;
                    if on {
                        next_frame = Instant::now();
                        window_start = Instant::now();
                        window_frames = 0;
                        stat(&|stats| stats.late_frames = 0);
                    } else if let Some(target) = target.as_ref() {
                        target.monitor().shrink();
                    }
                }
                Msg::ResetStats => {
                    scrub_total = 0.0;
                    stat(&|stats| {
                        *stats = Stats {
                            renderer: stats.renderer.clone(),
                            ..Stats::default()
                        }
                    });
                }
            }
        }

        let (Some(drawing), Some(doc)) = (target.as_mut(), doc.as_ref()) else {
            continue;
        };
        let fps = (doc.settings.rate_num as f64 / doc.settings.rate_den.max(1) as f64).max(1.0);
        let interval = Duration::from_secs_f64(1.0 / fps);

        if playing {
            let now = Instant::now();
            if now < next_frame {
                continue;
            }
            let at = playback.position_now();
            if at >= duration {
                playback.pause();
                playing = false;
                pending = Some((duration, Instant::now()));
                continue;
            }
            let drawn = drawing.draw(doc, at, true);
            drawing.prefetch(doc, at);
            let (left, right) = playback.take_levels();
            let peak = f64::from(left.max(right));
            stat(&|stats| stats.audio_peak = stats.audio_peak.max(peak));
            window_frames += 1;
            let mut late = 0u64;
            next_frame += interval;
            let after = Instant::now();
            if next_frame < after {
                // Late: the frames missed are skipped, not queued.
                late = ((after - next_frame).as_secs_f64() / interval.as_secs_f64()).ceil() as u64;
                next_frame = after + interval;
            }
            let elapsed = window_start.elapsed();
            let window_fps = (elapsed >= Duration::from_secs(1)).then(|| {
                let rate = window_frames as f64 / elapsed.as_secs_f64();
                window_start = Instant::now();
                window_frames = 0;
                rate
            });
            stat(&|stats| {
                stats.late_frames += late;
                if let Some(rate) = window_fps {
                    stats.play_fps = rate;
                }
                note(stats, &drawn);
            });
        } else if let Some((at, since)) = pending.take() {
            let drawn = drawing.draw(doc, at, false);
            let waited = ms(since.elapsed());
            if drawn.is_ok() {
                scrub_total += waited;
            }
            let total = scrub_total;
            stat(&|stats| {
                if drawn.is_ok() {
                    stats.scrubs += 1;
                    stats.last_scrub_ms = waited;
                    stats.max_scrub_ms = stats.max_scrub_ms.max(waited);
                    stats.mean_scrub_ms = total / stats.scrubs as f64;
                }
                note(stats, &drawn);
            });
        }
    }
}

/// What one draw took, or why it did not happen.
pub(crate) type Drawn = Result<(Duration, Duration), String>;

fn note(stats: &mut Stats, drawn: &Drawn) {
    match drawn {
        Ok((decode, draw)) => {
            stats.frames += 1;
            stats.last_decode_ms = ms(*decode);
            stats.last_draw_ms = ms(*draw);
        }
        Err(error) => {
            if stats.last_error != *error {
                log::warn!("frame: {error}");
            }
            stats.last_error = error.clone();
        }
    }
}

fn spec(doc: &Doc, at: f64, size: (u32, u32), moving: bool) -> concat_host::preview::FrameSpec {
    concat_host::preview::FrameSpec {
        time: at,
        width: size.0,
        height: size.1,
        moving,
        proxy: false,
        color_space: doc.color,
        canvas: doc.settings.canvas,
    }
}

#[cfg(target_os = "android")]
mod target {
    use std::ptr::NonNull;
    use std::sync::Arc;
    use std::time::Instant;

    use concat_host::preview::Monitor;
    use ndk::hardware_buffer_format::HardwareBufferFormat;
    use ndk::native_window::NativeWindow;

    use super::{Doc, Drawn, fit, spec};

    /// The device, kept across surfaces: a phone backgrounded and brought
    /// back gets a new Surface, not a new GPU.
    struct Device {
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        monitor: Monitor,
        backend: wgpu::Backends,
    }

    /// Kept between attaches on this thread only.
    static KEPT: std::sync::Mutex<Option<KeptDevice>> = std::sync::Mutex::new(None);
    struct KeptDevice(Device);

    pub enum Target {
        Gpu(Box<GpuTarget>),
        Cpu(CpuTarget),
    }

    pub struct GpuTarget {
        gpu: Device,
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        pipeline: wgpu::RenderPipeline,
        layout: wgpu::BindGroupLayout,
        sampler: wgpu::Sampler,
        described: String,
        // Last: the surface above must go before the window's reference.
        _window: NativeWindow,
    }

    pub struct CpuTarget {
        window: NativeWindow,
        monitor: Monitor,
        geometry: (u32, u32),
        size: (u32, u32),
    }

    const BLIT: &str = r#"
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) index: u32) -> Out {
    var corners = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    let at = corners[index];
    var out: Out;
    out.position = vec4(at, 0.0, 1.0);
    out.uv = vec2((at.x + 1.0) * 0.5, 1.0 - (at.y + 1.0) * 0.5);
    return out;
}
@fragment fn fs_plain(in: Out) -> @location(0) vec4<f32> {
    return textureSample(picture, linear_sampler, in.uv);
}
// An sRGB surface encodes on write; the picture is encoded already.
@fragment fn fs_linear(in: Out) -> @location(0) vec4<f32> {
    let c = textureSample(picture, linear_sampler, in.uv);
    return vec4(pow(c.rgb, vec3(2.2)), c.a);
}
"#;

    impl Target {
        pub fn attach(window: u64, width: u32, height: u32, mode: &str) -> Result<Target, String> {
            let pointer = NonNull::new(window as *mut ndk_sys::ANativeWindow)
                .ok_or_else(|| "no window".to_owned())?;
            // SAFETY: the Kotlin side hands over the reference
            // ANativeWindow_fromSurface acquired; this owns it now.
            let window = unsafe { NativeWindow::from_ptr(pointer) };
            match mode {
                "cpu" => Ok(Target::Cpu(CpuTarget {
                    window,
                    monitor: Monitor::new(),
                    geometry: (0, 0),
                    size: (width, height),
                })),
                "gl" => GpuTarget::new(window, width, height, wgpu::Backends::GL)
                    .map(|target| Target::Gpu(Box::new(target))),
                _ => GpuTarget::new(window.clone(), width, height, wgpu::Backends::VULKAN)
                    .or_else(|error| {
                        log::warn!(
                            "Vulkan would not draw on the surface ({error}); trying OpenGL ES"
                        );
                        GpuTarget::new(window, width, height, wgpu::Backends::GL)
                    })
                    .map(|target| Target::Gpu(Box::new(target))),
            }
        }

        pub fn describe(&self) -> String {
            match self {
                Target::Gpu(gpu) => gpu.described.clone(),
                Target::Cpu(_) => "cpu: Monitor::frame into ANativeWindow_lock".to_owned(),
            }
        }

        pub fn monitor(&self) -> &Monitor {
            match self {
                Target::Gpu(gpu) => &gpu.gpu.monitor,
                Target::Cpu(cpu) => &cpu.monitor,
            }
        }

        /// Gives the surface up, keeping the device for the next one.
        pub fn release_surface(self) -> Option<()> {
            if let Target::Gpu(gpu) = self {
                let GpuTarget { gpu, surface, .. } = *gpu;
                drop(surface);
                *KEPT.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    Some(KeptDevice(gpu));
            }
            Some(())
        }

        pub fn prefetch(&self, doc: &Doc, at: f64) {
            let size = match self {
                Target::Gpu(gpu) => fit(
                    (doc.settings.width, doc.settings.height),
                    (gpu.config.width, gpu.config.height),
                ),
                Target::Cpu(cpu) => fit((doc.settings.width, doc.settings.height), cpu.size),
            };
            self.monitor().prefetch(
                Arc::clone(&doc.clips),
                &doc.settings,
                spec(doc, at, size, true),
                8,
            );
        }

        pub fn draw(&mut self, doc: &Doc, at: f64, moving: bool) -> Drawn {
            match self {
                Target::Gpu(gpu) => gpu.draw(doc, at, moving),
                Target::Cpu(cpu) => cpu.draw(doc, at, moving),
            }
        }
    }

    impl GpuTarget {
        fn new(
            window: NativeWindow,
            width: u32,
            height: u32,
            backend: wgpu::Backends,
        ) -> Result<GpuTarget, String> {
            use raw_window_handle::{
                AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle,
            };
            let kept = KEPT
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .map(|kept| kept.0)
                .filter(|kept| kept.backend == backend);
            let instance = match &kept {
                Some(kept) => kept.instance.clone(),
                None => wgpu::Instance::new(wgpu::InstanceDescriptor {
                    backends: backend,
                    ..wgpu::InstanceDescriptor::new_without_display_handle()
                }),
            };
            // SAFETY: the window outlives the surface - both live in the
            // target, the surface dropped first.
            let surface = unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(
                        RawDisplayHandle::Android(AndroidDisplayHandle::new()),
                    ),
                    raw_window_handle: RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(
                        window.ptr().cast(),
                    )),
                })
            }
            .map_err(|error| format!("no {backend:?} surface: {error}"))?;
            let started = Instant::now();
            let gpu = match kept {
                Some(kept) if kept.adapter.is_surface_supported(&surface) => kept,
                _ => {
                    let adapter = pollster::block_on(instance.request_adapter(
                        &wgpu::RequestAdapterOptions {
                            power_preference: wgpu::PowerPreference::HighPerformance,
                            compatible_surface: Some(&surface),
                            ..Default::default()
                        },
                    ))
                    .map_err(|error| format!("no {backend:?} adapter: {error}"))?;
                    let (device, queue) =
                        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                            label: Some("concat spike"),
                            required_limits: adapter.limits(),
                            ..Default::default()
                        }))
                        .map_err(|error| format!("no device: {error}"))?;
                    device.on_uncaptured_error(Arc::new(|error: wgpu::Error| {
                        log::error!("GPU error: {error}");
                    }));
                    let monitor = Monitor::with_gpu(device.clone(), queue.clone());
                    Device {
                        instance: instance.clone(),
                        adapter,
                        device,
                        queue,
                        monitor,
                        backend,
                    }
                }
            };
            let info = gpu.adapter.get_info();
            let capabilities = surface.get_capabilities(&gpu.adapter);
            let format = capabilities
                .formats
                .iter()
                .copied()
                .find(|format| !format.is_srgb())
                .or_else(|| capabilities.formats.first().copied())
                .ok_or_else(|| "the surface offers no format".to_owned())?;
            let mut config = surface
                .get_default_config(&gpu.adapter, width.max(1), height.max(1))
                .ok_or_else(|| "the adapter cannot draw on this surface".to_owned())?;
            config.format = format;
            config.present_mode = wgpu::PresentMode::Fifo;
            surface.configure(&gpu.device, &config);

            let device = &gpu.device;
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("spike blit"),
                source: wgpu::ShaderSource::Wgsl(BLIT.into()),
            });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("spike blit"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("spike blit"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("spike blit"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if format.is_srgb() {
                        "fs_linear"
                    } else {
                        "fs_plain"
                    }),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("spike blit"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            let described = format!(
                "gpu: {} ({:?}, {:?}, {:?}) surface {}x{} up in {:.0} ms",
                info.name,
                info.backend,
                info.device_type,
                format,
                config.width,
                config.height,
                started.elapsed().as_secs_f64() * 1000.0
            );
            log::info!("{described}");
            Ok(GpuTarget {
                gpu,
                surface,
                config,
                pipeline,
                layout,
                sampler,
                described,
                _window: window,
            })
        }

        fn draw(&mut self, doc: &Doc, at: f64, moving: bool) -> Drawn {
            let size = fit(
                (doc.settings.width, doc.settings.height),
                (self.config.width, self.config.height),
            );
            let spec = spec(doc, at, size, moving);
            let started = Instant::now();
            let sources =
                self.gpu
                    .monitor
                    .frame_sources(Arc::clone(&doc.clips), &doc.settings, spec)?;
            let decoded = started.elapsed();
            let drawing = Instant::now();
            let texture = self.gpu.monitor.texture_of(&sources, spec)?;
            let frame = match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    self.surface.configure(&self.gpu.device, &self.config);
                    return Err("the surface was outdated; reconfigured".to_owned());
                }
                other => return Err(format!("no surface texture: {other:?}")),
            };
            let view = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let picture = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind = self
                .gpu
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("spike blit"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&picture),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
            let mut encoder =
                self.gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("spike blit"),
                    });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("spike blit"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                // Letterboxed: the frame's shape, centred in the surface.
                let (sw, sh) = (self.config.width as f32, self.config.height as f32);
                let aspect = doc.settings.width.max(1) as f32 / doc.settings.height.max(1) as f32;
                let (w, h) = if sw / sh > aspect {
                    (sh * aspect, sh)
                } else {
                    (sw, sw / aspect)
                };
                pass.set_viewport((sw - w) / 2.0, (sh - h) / 2.0, w, h, 0.0, 1.0);
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.draw(0..3, 0..1);
            }
            self.gpu.queue.submit([encoder.finish()]);
            self.gpu.queue.present(frame);
            Ok((decoded, drawing.elapsed()))
        }
    }

    impl CpuTarget {
        fn draw(&mut self, doc: &Doc, at: f64, moving: bool) -> Drawn {
            let size = fit((doc.settings.width, doc.settings.height), self.size);
            let spec = spec(doc, at, size, moving);
            let started = Instant::now();
            let rgba = self
                .monitor
                .frame(Arc::clone(&doc.clips), &doc.settings, spec)?;
            let decoded = started.elapsed();
            let drawing = Instant::now();
            if self.geometry != size {
                self.window
                    .set_buffers_geometry(
                        size.0 as i32,
                        size.1 as i32,
                        Some(HardwareBufferFormat::R8G8B8A8_UNORM),
                    )
                    .map_err(|error| format!("set geometry: {error}"))?;
                self.geometry = size;
            }
            let mut buffer = self
                .window
                .lock(None)
                .map_err(|error| format!("lock: {error}"))?;
            let (width, height, stride) = (buffer.width(), buffer.height(), buffer.stride());
            let row = (size.0 as usize).min(width) * 4;
            let bits = buffer.bits().cast::<u8>();
            for y in 0..(size.1 as usize).min(height) {
                let from = &rgba[y * size.0 as usize * 4..][..row];
                // SAFETY: the locked buffer is stride * height pixels of
                // four bytes; `row` is at most `width` pixels of a line.
                unsafe {
                    std::ptr::copy_nonoverlapping(from.as_ptr(), bits.add(y * stride * 4), row);
                }
            }
            drop(buffer);
            Ok((decoded, drawing.elapsed()))
        }
    }
}

/// Off Android there is no Surface to draw on; the engine still opens,
/// imports and plays, which is what the host tests exercise.
#[cfg(not(target_os = "android"))]
mod target {
    use concat_host::preview::Monitor;

    use super::{Doc, Drawn};

    pub enum Target {}

    impl Target {
        pub fn attach(_: u64, _: u32, _: u32, _: &str) -> Result<Target, String> {
            Err("there is no Surface to draw on off Android".to_owned())
        }
        pub fn describe(&self) -> String {
            match *self {}
        }
        pub fn monitor(&self) -> &Monitor {
            match *self {}
        }
        pub fn release_surface(self) -> Option<()> {
            match self {}
        }
        pub fn prefetch(&self, _: &Doc, _: f64) {
            match *self {}
        }
        pub fn draw(&mut self, _: &Doc, _: f64, _: bool) -> Drawn {
            match *self {}
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_frame_fits_its_surface_without_growing() {
        assert_eq!(super::fit((1920, 1080), (1080, 2000)), (1080, 608));
        assert_eq!(super::fit((1080, 1920), (1080, 1200)), (674, 1200));
        assert_eq!(super::fit((640, 360), (1080, 2000)), (640, 360));
    }
}

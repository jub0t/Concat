// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The one GPU device the window and the engine share.
//!
//! Slint renders the window on it, and the monitor composites on it. Because
//! both sides hold the same device, a composited frame is a texture the
//! renderer samples directly: nothing is read back and nothing is copied.
//! Created before the backend is selected, because Slint takes the device at
//! selection time; `None` when the machine offers no adapter, and then the
//! window renders the way it did without one.
//!
//! On Android the backend owns its device and none is shared, so the type
//! is carried and never opened there; see platform.rs.
#![cfg_attr(target_os = "android", allow(dead_code))]

use std::sync::OnceLock;

/// The adapter this run opened, as one line for Settings > About: name,
/// backend, and whether it is a software rasteriser. Written once by
/// [`Gpu::acquire`], because the adapter itself is moved into the monitor
/// before the About sheet is built.
static ADAPTER: OnceLock<String> = OnceLock::new();

/// What [`Gpu::acquire`] found, or `None` when it found nothing.
pub fn adapter_description() -> Option<&'static str> {
    ADAPTER.get().map(String::as_str)
}

/// The shared device and what it was created from.
#[derive(Clone)]
pub struct Gpu {
    /// The instance, which Slint needs to make a surface for the window.
    pub instance: wgpu::Instance,
    /// The adapter, which Slint reads the backend off.
    pub adapter: wgpu::Adapter,
    /// The device both sides draw on.
    pub device: wgpu::Device,
    /// The one queue, so submissions from both sides are ordered.
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Opens the machine's best adapter and a device on it.
    pub fn acquire() -> Option<Gpu> {
        // Only the backend Slint's Skia renderer can share a device on:
        // Skia is built against the platform's own API, so a device on any
        // other backend is refused at selection time. PRIMARY let wgpu pick
        // Vulkan on Windows machines that offer it, and the window then
        // failed to open with "Unsupported WGPU backend for use with Skia".
        let backends = if cfg!(target_vendor = "apple") {
            wgpu::Backends::METAL
        } else if cfg!(windows) {
            wgpu::Backends::DX12
        } else {
            wgpu::Backends::VULKAN
        };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        // This is not always a GPU. wgpu ranks a CPU adapter last but never
        // leaves it out, so on a machine whose hardware it will not drive -
        // its D3D12 backend wants resource binding tier 2, which Intel's
        // Broadwell-era chips do not have - the adapter that comes back is
        // WARP, Windows' software rasteriser. The window still opens on it;
        // see `is_software` and platform.rs for what that changes.
        // https://github.com/jub0t/Concat/issues/135
        let info = adapter.get_info();
        log::info!(
            "GPU adapter: {} ({:?}, {:?}, driver {})",
            info.name,
            info.device_type,
            info.backend,
            info.driver_info
        );
        let _ = ADAPTER.set(if info.device_type == wgpu::DeviceType::Cpu {
            format!("{} · {} · software rasteriser", info.name, info.backend)
        } else {
            format!("{} · {}", info.name, info.backend)
        });
        // The adapter's own limits, not `Limits::default()`: the defaults are
        // the downlevel floor every GL-class device can meet, and their
        // `max_texture_dimension_2d` is 8192 whatever the machine can do -
        // 16384 on any current desktop GPU. Both sides of this device want
        // the real number: the monitor composites the timeline's frame, and
        // Slint's renderer allocates a layer as large as the element it is
        // flattening.
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("concat"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .ok()?;
        Some(Gpu {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// Whether the adapter is a software rasteriser - WARP on Windows,
    /// llvmpipe on Linux - rather than a GPU. Everything still draws, only
    /// slower, and the sysinfo sheet says so.
    pub fn is_software(&self) -> bool {
        self.adapter.get_info().device_type == wgpu::DeviceType::Cpu
    }

    /// The device as Slint takes it.
    pub fn configuration(&self) -> slint::wgpu_29::WGPUConfiguration {
        slint::wgpu_29::WGPUConfiguration::Manual {
            instance: self.instance.clone(),
            adapter: self.adapter.clone(),
            device: self.device.clone(),
            queue: self.queue.clone(),
        }
    }
}

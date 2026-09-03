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
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("concat"),
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

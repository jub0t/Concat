// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What the GPU is, not what it draws.
//!
//! [`WgpuCompositor`](crate::WgpuCompositor) asks wgpu for the best adapter
//! and never looks at it again once it has a device. This module reads the
//! same adapter for one more fact - discrete, integrated, or nothing at all
//! - so a caller with no compositor yet (or none ever, on a headless build)
//! can still know roughly what the machine offers. See `concat_host::hardware`
//! for where that fact becomes a quality tier.

/// The rough shape of a GPU adapter, the one distinction that actually
/// predicts how much rendering headroom the machine has: a discrete card, an
/// integrated one sharing system memory, or nothing real to render on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterKind {
    /// A dedicated GPU with its own memory.
    Discrete,
    /// A GPU sharing the CPU's package and memory.
    Integrated,
    /// A virtualised adapter - a VM or a remote desktop.
    Virtual,
    /// wgpu's own software fallback: no real GPU at all.
    Cpu,
    /// Reported, but not one of the above.
    Other,
}

/// Reads the kind out of an adapter wgpu (or a caller who already acquired
/// one) has already described. The one place this crate reads
/// [`wgpu::DeviceType`], so nothing else has to know its variants.
#[cfg(feature = "gpu")]
pub fn kind_of(info: &wgpu::AdapterInfo) -> AdapterKind {
    match info.device_type {
        wgpu::DeviceType::DiscreteGpu => AdapterKind::Discrete,
        wgpu::DeviceType::IntegratedGpu => AdapterKind::Integrated,
        wgpu::DeviceType::VirtualGpu => AdapterKind::Virtual,
        wgpu::DeviceType::Cpu => AdapterKind::Cpu,
        wgpu::DeviceType::Other => AdapterKind::Other,
    }
}

/// The same request [`WgpuCompositor::new`](crate::WgpuCompositor::new)
/// makes, but for a caller with no window and no compositor of its own - the
/// CLI, or a headless host probing the machine before anything else opens a
/// device. Adapter only: nothing here is kept, so this costs a request and
/// nothing more.
#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
pub fn probe_adapter() -> Option<(AdapterKind, String)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .ok()?;
    let info = adapter.get_info();
    Some((kind_of(&info), info.name))
}

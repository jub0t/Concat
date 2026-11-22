// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A GPU's memory as its driver tells it: every heap's size, the budget
//! the driver gives this process in it, and what the process uses of it.
//!
//! Every device Concat opens on Direct3D 12 or Vulkan carries wgpu's memory
//! gates ([`crate::gpu::MEMORY_BUDGET`]): a texture or buffer is refused as
//! out of memory once the driver's budget is 95% spent, and the device is
//! lost at 99%. They stopped full 8 GB cards hanging exports (#223, #297).
//! Until now nothing said the figures they compare, so a report of "out of
//! memory" could not be told from a full card (#802). [`Memory`] reads
//! them, a line a heap, as each device opens and at its first refusal
//! ([`ran_out`]).
//!
//! What the gates compare, read from wgpu-hal 30:
//!
//! - **Vulkan** picks the heap of the first memory type with the flags the
//!   allocator *prefers* for the request - device-local for a texture,
//!   device-local and host-visible for a buffer the CPU writes - and
//!   refuses unless that heap's usage plus the request is under 95% of its
//!   budget, both from `VK_EXT_memory_budget`. Device loss is any heap past
//!   99%. On a card without resizable BAR, the preferred heap for a buffer
//!   the CPU writes is the BAR: the 256 MB window the CPU maps onto the
//!   card, shared by every program, whose budget of 88-120 MB is likely
//!   true. The allocator puts such a buffer in system memory when the
//!   window is full; the published gate refused it instead, and lost the
//!   device over the window at the next submit (#746). Concat's copy of
//!   wgpu-hal (src/patches/wgpu-hal) gates such a buffer on the heap it
//!   would land in, and leaves the window out of the loss gate.
//! - **Direct3D 12** asks `QueryVideoMemoryInfo` for the local segment
//!   (textures; everything on an integrated GPU) or the non-local one
//!   (buffers the CPU reads or writes on a card, which live in system
//!   memory), and refuses once the process's usage plus the larger of the
//!   request and a whole allocator block - 128 MB on the card, 64 MB in
//!   system memory - reaches 95% of the budget. Device loss is either
//!   segment past 99%.
//! - **Metal** and OpenGL ES have no gates; Metal's figures are said all the
//!   same, so a report from a Mac reads like one from anywhere else.
//!
//! A figure the driver does not give is "unknown", never a guess and never
//! a panic.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Where a heap's memory is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// On the card, out of the CPU's reach.
    Card,
    /// On the card, with memory types the CPU can map as well: the whole
    /// card on an integrated GPU or with resizable BAR.
    Mapped,
    /// The BAR: a small heap on the card that only memory the CPU maps
    /// lives in, beside a larger one that is the card's own. 256 MB on most
    /// cards without resizable BAR, shared by every program.
    Bar,
    /// In system memory.
    System,
    /// Shared with the system, as on Apple silicon.
    Unified,
}

impl Kind {
    /// The kind as the log line says it.
    fn words(self) -> &'static str {
        match self {
            Kind::Card => "on the card",
            Kind::Mapped => "on the card, mapped for the CPU",
            Kind::Bar => "the CPU's window onto the card (BAR)",
            Kind::System => "in system memory",
            Kind::Unified => "shared with the system",
        }
    }
}

/// One memory heap, as the driver reports it. A figure the driver does not
/// give, or gives as zero, is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heap {
    /// The API's name for it: `heap 2`, `local`, `working set`.
    pub name: String,
    /// Where it is.
    pub kind: Kind,
    /// How big it is.
    pub size: Option<u64>,
    /// How much of it the driver says this process may use.
    pub budget: Option<u64>,
    /// How much of it this process uses now.
    pub usage: Option<u64>,
}

/// An adapter's memory at one moment: every heap its API reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Memory {
    /// The adapter's name.
    pub adapter: String,
    /// The API it was read through.
    pub backend: wgpu::Backend,
    /// Its heaps; empty where the API reports none.
    pub heaps: Vec<Heap>,
}

impl Memory {
    /// The memory of `device`'s adapter, read through the device's API.
    pub fn of_device(device: &wgpu::Device) -> Self {
        let info = device.adapter_info();
        let heaps = match info.backend {
            wgpu::Backend::Vulkan => vulkan::of_device(device),
            wgpu::Backend::Dx12 => d3d12::of_device(device),
            wgpu::Backend::Metal => metal::of_device(device),
            _ => None,
        };
        Self {
            adapter: info.name,
            backend: info.backend,
            heaps: heaps.unwrap_or_default(),
        }
    }

    /// The memory of `adapter`: as a device opens on it, or when none would.
    pub fn of_adapter(adapter: &wgpu::Adapter) -> Self {
        let info = adapter.get_info();
        let heaps = match info.backend {
            wgpu::Backend::Vulkan => vulkan::of_adapter(adapter),
            wgpu::Backend::Dx12 => d3d12::of_adapter(adapter),
            wgpu::Backend::Metal => metal::of_adapter(adapter),
            _ => None,
        };
        Self {
            adapter: info.name,
            backend: info.backend,
            heaps: heaps.unwrap_or_default(),
        }
    }

    /// The log's lines for it, one a heap, `when` saying the moment (`as
    /// the window's device opened`).
    pub fn lines(&self, when: &str) -> Vec<String> {
        let head = format!(
            "gpu: memory of {} ({:?}) {when}",
            self.adapter, self.backend
        );
        if self.heaps.is_empty() {
            return vec![format!("{head} · unknown: the API reports no heaps")];
        }
        let figure = |value: Option<u64>| value.map_or("unknown".to_owned(), concat_log::bytes);
        self.heaps
            .iter()
            .map(|heap| {
                format!(
                    "{head} · {}, {} · size {} · budget {} · Concat uses {}",
                    heap.name,
                    heap.kind.words(),
                    figure(heap.size),
                    figure(heap.budget),
                    figure(heap.usage),
                )
            })
            .collect()
    }

    /// Writes [`Memory::lines`] to the log at `level`.
    pub fn say(&self, level: log::Level, when: &str) {
        for line in self.lines(when) {
            log::log!(level, "{line}");
        }
    }

    /// The card's own memory: the largest heap on the card, mapped for the
    /// CPU or not, with a budget the driver reports. `None` on a GPU with
    /// no memory of its own (integrated, Apple's) or one whose API says
    /// nothing of budgets.
    pub fn card(&self) -> Option<&Heap> {
        self.heaps
            .iter()
            .filter(|heap| matches!(heap.kind, Kind::Card | Kind::Mapped))
            .filter(|heap| heap.budget.is_some())
            .max_by_key(|heap| heap.size.or(heap.budget))
    }
}

/// How many times a device of this process has run out of memory: what
/// the caches on the card read to know they must hold less. It only ever
/// goes up.
static PRESSURE: AtomicU64 = AtomicU64::new(0);

/// See [`PRESSURE`]: compare with the figure last read, and a change means
/// a device has run out since.
pub fn pressure() -> u64 {
    PRESSURE.load(Ordering::Relaxed)
}

/// Says `memory` once a device - `owner`, as the log names it - has run
/// out of memory. `said` is the device's own flag, shared by everything
/// that draws on it, so only its first refusal is said: what an export
/// refused fifty times needs is the figures behind the first, not fifty
/// copies. `memory` is read only then.
///
/// Every call counts as [`pressure`], said or not.
pub fn ran_out(said: &AtomicBool, owner: &str, memory: impl FnOnce() -> Memory) {
    PRESSURE.fetch_add(1, Ordering::Relaxed);
    if !said.swap(true, Ordering::Relaxed) {
        memory().say(log::Level::Warn, &format!("as {owner} ran out of memory"));
    }
}

/// Whether a message from wgpu says a device ran out of memory: its
/// device-loss gate's ("Out of memory") and a refused device's ("Not enough
/// memory left.").
pub fn says_out_of_memory(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("out of memory") || message.contains("not enough memory")
}

/// What a Vulkan heap is, from whether it is device-local, how many memory
/// types live on it and how many of those the CPU can map, its size, and
/// the size of the largest device-local heap. The BAR is told from a card
/// whose whole memory is mappable by both: every type on it mappable, and a
/// larger device-local heap beside it.
#[cfg_attr(
    not(any(windows, target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
fn vulkan_kind(local: bool, types: usize, mapped: usize, size: u64, largest_local: u64) -> Kind {
    match (local, mapped) {
        (false, _) => Kind::System,
        (true, 0) => Kind::Card,
        (true, _) if mapped == types && size < largest_local => Kind::Bar,
        (true, _) => Kind::Mapped,
    }
}

/// Vulkan's figures, from `VK_EXT_memory_budget` where the device has it.
#[cfg(any(windows, target_os = "linux", target_os = "android"))]
mod vulkan {
    use super::{Heap, vulkan_kind};
    use ash::vk;
    use std::ffi::CStr;

    pub(super) fn of_device(device: &wgpu::Device) -> Option<Vec<Heap>> {
        // SAFETY: the raw handles are only asked questions about the
        // physical device, which Vulkan answers on any thread without
        // external synchronisation; nothing is created or destroyed, and the
        // guard is dropped before this returns.
        let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }?;
        let shared = hal.shared_instance();
        Some(heaps(
            shared.entry(),
            shared.raw_instance(),
            shared.instance_api_version(),
            shared.extensions(),
            hal.raw_physical_device(),
        ))
    }

    pub(super) fn of_adapter(adapter: &wgpu::Adapter) -> Option<Vec<Heap>> {
        // SAFETY: as `of_device`'s.
        let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
        let shared = hal.shared_instance();
        Some(heaps(
            shared.entry(),
            shared.raw_instance(),
            shared.instance_api_version(),
            shared.extensions(),
            hal.raw_physical_device(),
        ))
    }

    /// Every heap of `physical`, with its budget and usage where the
    /// device has the budget extension and the instance can ask for it
    /// (Vulkan 1.1, or `VK_KHR_get_physical_device_properties2`): the same
    /// figures wgpu's gates read, read the same way.
    fn heaps(
        entry: &ash::Entry,
        instance: &ash::Instance,
        api: u32,
        extensions: &[&'static CStr],
        physical: vk::PhysicalDevice,
    ) -> Vec<Heap> {
        // SAFETY: `physical` is a handle of `instance`, alive while the
        // caller holds the hal guard.
        let supported =
            unsafe { instance.enumerate_device_extension_properties(physical) }.unwrap_or_default();
        let has_budget = supported.iter().any(|extension| {
            extension.extension_name_as_c_str() == Ok(ash::ext::memory_budget::NAME)
        });
        // SAFETY: as above.
        let device_api = unsafe { instance.get_physical_device_properties(physical) }.api_version;
        let mut budget = vk::PhysicalDeviceMemoryBudgetPropertiesEXT::default();
        let mut read_budget = false;
        let base = {
            let mut properties = vk::PhysicalDeviceMemoryProperties2::default();
            if has_budget {
                properties = properties.push_next(&mut budget);
                if api >= vk::API_VERSION_1_1 && device_api >= vk::API_VERSION_1_1 {
                    // SAFETY: both are 1.1, where the call is core, and the
                    // chain is the one the budget extension defines.
                    unsafe {
                        instance.get_physical_device_memory_properties2(physical, &mut properties)
                    };
                    read_budget = true;
                } else if extensions.contains(&ash::khr::get_physical_device_properties2::NAME) {
                    let khr =
                        ash::khr::get_physical_device_properties2::Instance::new(entry, instance);
                    // SAFETY: the instance enabled the extension, as wgpu's
                    // own budget read requires.
                    unsafe {
                        khr.get_physical_device_memory_properties2(physical, &mut properties)
                    };
                    read_budget = true;
                }
            }
            if read_budget {
                properties.memory_properties
            } else {
                // SAFETY: as above.
                unsafe { instance.get_physical_device_memory_properties(physical) }
            }
        };
        let types =
            &base.memory_types[..(base.memory_type_count as usize).min(vk::MAX_MEMORY_TYPES)];
        let count = (base.memory_heap_count as usize).min(vk::MAX_MEMORY_HEAPS);
        let heaps = &base.memory_heaps[..count];
        let local = |heap: &vk::MemoryHeap| heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL);
        let largest_local = heaps
            .iter()
            .filter(|heap| local(heap))
            .map(|heap| heap.size)
            .max()
            .unwrap_or(0);
        heaps
            .iter()
            .enumerate()
            .map(|(index, heap)| {
                let on_it = types
                    .iter()
                    .filter(|kind| kind.heap_index as usize == index);
                let mapped = on_it
                    .clone()
                    .filter(|kind| {
                        kind.property_flags
                            .contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
                    })
                    .count();
                Heap {
                    name: format!("heap {index}"),
                    kind: vulkan_kind(local(heap), on_it.count(), mapped, heap.size, largest_local),
                    size: Some(heap.size).filter(|&size| size > 0),
                    budget: read_budget
                        .then(|| budget.heap_budget[index])
                        .filter(|&budget| budget > 0),
                    usage: read_budget.then(|| budget.heap_usage[index]),
                }
            })
            .collect()
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
mod vulkan {
    use super::Heap;

    pub(super) fn of_device(_: &wgpu::Device) -> Option<Vec<Heap>> {
        None
    }

    pub(super) fn of_adapter(_: &wgpu::Adapter) -> Option<Vec<Heap>> {
        None
    }
}

/// Direct3D 12's figures, from DXGI: the card's dedicated memory, and each
/// segment's budget and usage.
#[cfg(windows)]
mod d3d12 {
    use super::{Heap, Kind};
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_MEMORY_SEGMENT_GROUP, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
        DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3,
        IDXGIFactory4,
    };

    pub(super) fn of_device(device: &wgpu::Device) -> Option<Vec<Heap>> {
        // SAFETY: the raw device is only asked which adapter it is on, and
        // DXGI then only how much memory that adapter has; nothing is
        // created on the device, and the guard is dropped before this
        // returns.
        let hal = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }?;
        let luid = unsafe { hal.raw_device().GetAdapterLuid() };
        let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.ok()?;
        let adapter: IDXGIAdapter3 = unsafe { factory.EnumAdapterByLuid(luid) }.ok()?;
        Some(heaps(&adapter))
    }

    pub(super) fn of_adapter(adapter: &wgpu::Adapter) -> Option<Vec<Heap>> {
        // SAFETY: as `of_device`'s.
        let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Dx12>() }?;
        let raw: &IDXGIAdapter3 = hal.raw_adapter();
        Some(heaps(raw))
    }

    /// The local segment (the card; all of it on an integrated GPU, whose
    /// dedicated memory is a token figure) and the non-local one (system
    /// memory the card reaches), as wgpu's gate reads them.
    fn heaps(adapter: &IDXGIAdapter3) -> Vec<Heap> {
        // SAFETY: a description and two queries, which DXGI answers on any
        // thread.
        let description = unsafe { adapter.GetDesc() }.ok();
        let query = |group: DXGI_MEMORY_SEGMENT_GROUP| {
            let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
            unsafe { adapter.QueryVideoMemoryInfo(0, group, &mut info) }
                .ok()
                .map(|()| info)
        };
        let local = query(DXGI_MEMORY_SEGMENT_GROUP_LOCAL);
        let shared = query(DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL);
        let size = |bytes: usize| Some(bytes as u64).filter(|&bytes| bytes > 0);
        vec![
            Heap {
                name: "local".to_owned(),
                kind: Kind::Card,
                size: description.and_then(|desc| size(desc.DedicatedVideoMemory)),
                budget: local.map(|info| info.Budget).filter(|&budget| budget > 0),
                usage: local.map(|info| info.CurrentUsage),
            },
            Heap {
                name: "non-local".to_owned(),
                kind: Kind::System,
                size: description.and_then(|desc| size(desc.SharedSystemMemory)),
                budget: shared.map(|info| info.Budget).filter(|&budget| budget > 0),
                usage: shared.map(|info| info.CurrentUsage),
            },
        ]
    }
}

#[cfg(not(windows))]
mod d3d12 {
    use super::Heap;

    pub(super) fn of_device(_: &wgpu::Device) -> Option<Vec<Heap>> {
        None
    }

    pub(super) fn of_adapter(_: &wgpu::Adapter) -> Option<Vec<Heap>> {
        None
    }
}

/// Metal's figures: the working set the system recommends and what the
/// device has allocated.
#[cfg(target_vendor = "apple")]
mod metal {
    use super::{Heap, Kind};
    use objc2_metal::MTLDevice;

    pub(super) fn of_device(device: &wgpu::Device) -> Option<Vec<Heap>> {
        // SAFETY: three properties read off the device, which Metal answers
        // on any thread; the guard is dropped before this returns.
        let hal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }?;
        let raw = hal.raw_device();
        Some(heaps(
            raw.hasUnifiedMemory(),
            raw.recommendedMaxWorkingSetSize(),
            raw.currentAllocatedSize(),
        ))
    }

    pub(super) fn of_adapter(adapter: &wgpu::Adapter) -> Option<Vec<Heap>> {
        // SAFETY: as `of_device`'s.
        let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Metal>() }?;
        let raw = hal.raw_device();
        Some(heaps(
            raw.hasUnifiedMemory(),
            raw.recommendedMaxWorkingSetSize(),
            raw.currentAllocatedSize(),
        ))
    }

    /// The one heap Metal speaks of, from what the device said.
    fn heaps(unified: bool, working_set: u64, allocated: usize) -> Vec<Heap> {
        vec![Heap {
            name: "working set".to_owned(),
            kind: if unified { Kind::Unified } else { Kind::Card },
            size: None,
            budget: Some(working_set).filter(|&budget| budget > 0),
            usage: Some(allocated as u64),
        }]
    }
}

#[cfg(not(target_vendor = "apple"))]
mod metal {
    use super::Heap;

    pub(super) fn of_device(_: &wgpu::Device) -> Option<Vec<Heap>> {
        None
    }

    pub(super) fn of_adapter(_: &wgpu::Adapter) -> Option<Vec<Heap>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * MB;

    /// The BAR is the small heap only mappable memory lives in, beside the
    /// card's own: NVIDIA's heap 2 of 246 MB next to heap 0 (#746). With
    /// resizable BAR, or on an integrated GPU, the card's one heap carries
    /// mappable types too, and is not the BAR.
    #[test]
    fn the_bar_is_told_from_a_mappable_card() {
        assert_eq!(vulkan_kind(true, 1, 1, 246 * MB, 2 * GB), Kind::Bar);
        assert_eq!(vulkan_kind(true, 7, 0, 2 * GB, 2 * GB), Kind::Card);
        assert_eq!(vulkan_kind(true, 4, 2, 8 * GB, 8 * GB), Kind::Mapped);
        // An integrated GPU's one heap, every type on it mappable.
        assert_eq!(vulkan_kind(true, 3, 3, 4 * GB, 4 * GB), Kind::Mapped);
        assert_eq!(vulkan_kind(false, 3, 3, 16 * GB, 2 * GB), Kind::System);
    }

    #[test]
    fn a_line_a_heap_and_unknown_said_as_such() {
        let heap = |name: &str, kind, size, budget, usage| Heap {
            name: name.to_owned(),
            kind,
            size: Some(size),
            budget,
            usage: Some(usage),
        };
        // #746's figures for heap 2: 137,756,672 bytes used of a
        // 120,193,024-byte budget.
        let memory = Memory {
            adapter: "NVIDIA GeForce MX150".to_owned(),
            backend: wgpu::Backend::Vulkan,
            heaps: vec![
                heap("heap 0", Kind::Card, 2 * GB, Some(1536 * MB), 90 * MB),
                heap("heap 1", Kind::System, 8 * GB, None, 30 * MB),
                heap(
                    "heap 2",
                    Kind::Bar,
                    246 * MB,
                    Some(120_193_024),
                    137_756_672,
                ),
            ],
        };
        let lines = memory.lines("as the window's device opened");
        assert_eq!(
            lines,
            vec![
                "gpu: memory of NVIDIA GeForce MX150 (Vulkan) as the window's device opened · \
                 heap 0, on the card · size 2.00 GB · budget 1.50 GB · Concat uses 90.0 MB",
                "gpu: memory of NVIDIA GeForce MX150 (Vulkan) as the window's device opened · \
                 heap 1, in system memory · size 8.00 GB · budget unknown · Concat uses 30.0 MB",
                "gpu: memory of NVIDIA GeForce MX150 (Vulkan) as the window's device opened · \
                 heap 2, the CPU's window onto the card (BAR) · size 246.0 MB · budget 114.6 MB · \
                 Concat uses 131.4 MB",
            ]
        );
        let none = Memory {
            adapter: "Adreno".to_owned(),
            backend: wgpu::Backend::Gl,
            heaps: Vec::new(),
        };
        assert_eq!(
            none.lines("as it opened"),
            vec!["gpu: memory of Adreno (Gl) as it opened · unknown: the API reports no heaps"]
        );
    }

    /// Only a device's first refusal is said.
    #[test]
    fn the_figures_are_said_once_a_device() {
        let said = AtomicBool::new(false);
        let mut read = 0;
        for _ in 0..3 {
            ran_out(&said, "a test's device", || {
                read += 1;
                Memory {
                    adapter: "test".to_owned(),
                    backend: wgpu::Backend::Noop,
                    heaps: Vec::new(),
                }
            });
        }
        assert_eq!(read, 1);
    }

    #[test]
    fn the_device_loss_gate_message_is_out_of_memory() {
        assert!(says_out_of_memory("Out of memory"));
        assert!(says_out_of_memory("Not enough memory left."));
        assert!(!says_out_of_memory("Device lost: driver internal error"));
    }
}

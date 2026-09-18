// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What this machine can comfortably do, in one word.
//!
//! `test/hardware_analysis/analyze_hardware.py` was the research prototype:
//! CPU, RAM and GPU probed by shelling out to `psutil`/`sysctl`/`nvidia-smi`,
//! scored, and read once by a person deciding what a first-run default
//! should be. This module is that decision made by the engine itself, on
//! every machine Concat starts on - CPU and RAM through `sysinfo`, the GPU
//! through whichever adapter the window already asked wgpu for (or, headless,
//! a probe of its own; see [`concat_render::hardware`]). It answers once,
//! `concat::studio` caches the answer in `settings.json`, and the answer only
//! ever sets a *default* - `quality_of`'s per-timeline override and the
//! export sheet's own picker both still win over it.
//!
//! What this module does not do, on purpose: read true VRAM size (wgpu has
//! no cross-platform query for it - the Python prototype's finer
//! `best_vram` bands collapse here to "discrete or not"), enumerate more
//! than the one adapter the caller already has or can cheaply ask for, or
//! detect CPU instruction sets. Those are real gaps, left for later.

use serde::{Deserialize, Serialize};
use sysinfo::System;

pub use concat_render::hardware::AdapterKind;

/// A rough performance tier, the one fact `concat::studio` actually acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// A machine to keep light: lower preview quality, smaller models.
    Low,
    /// A typical machine: today's defaults.
    Mainstream,
    /// Headroom to spare: higher preview quality, larger models.
    High,
    /// The top of the machines Concat sees: nothing held back by default.
    Enthusiast,
}

/// What was found, and the tier it adds up to.
#[derive(Clone, Debug, PartialEq)]
pub struct HardwareProfile {
    /// Logical CPU threads, from `sysinfo`.
    pub cpu_threads: usize,
    /// System memory, in gigabytes.
    pub ram_gb: f32,
    /// The adapter's kind, when one was found - by the caller's own probe,
    /// when it has one, or this module's headless fallback otherwise.
    pub gpu: Option<AdapterKind>,
    /// The adapter's name, for a report; not read for scoring.
    pub gpu_name: Option<String>,
    /// The tier the facts above add up to.
    pub tier: Tier,
}

/// Reads the machine and scores it. `gpu_hint` is a caller's own adapter -
/// the window always has one by the time it asks - and `None` falls back to
/// [`concat_render::hardware::probe_adapter`] under the crate's `gpu`
/// feature, so a build without it reports no GPU rather than lying about
/// one.
pub fn detect(gpu_hint: Option<(AdapterKind, String)>) -> HardwareProfile {
    let mut system = System::new();
    system.refresh_memory();
    system.refresh_cpu_all();

    let cpu_threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let ram_gb = system.total_memory() as f32 / (1024.0 * 1024.0 * 1024.0);

    #[cfg(feature = "gpu")]
    let gpu_hint = gpu_hint.or_else(concat_render::hardware::probe_adapter);
    let (gpu, gpu_name) = match gpu_hint {
        Some((kind, name)) => (Some(kind), Some(name)),
        None => (None, None),
    };

    HardwareProfile {
        cpu_threads,
        ram_gb,
        gpu,
        gpu_name,
        tier: classify(cpu_threads, ram_gb, gpu),
    }
}

/// The Python prototype's `classify_hardware`, ported: the same three
/// bands and thresholds for CPU threads and RAM, the same tier cutoffs.
/// The GPU band is coarser here (`Discrete` +4, `Integrated` +2, everything
/// else +0) because this crate has no VRAM size to weigh, as the module doc
/// explains.
fn classify(cpu_threads: usize, ram_gb: f32, gpu: Option<AdapterKind>) -> Tier {
    let mut score = 0;

    if cpu_threads >= 16 {
        score += 3;
    } else if cpu_threads >= 8 {
        score += 2;
    } else if cpu_threads >= 4 {
        score += 1;
    }

    if ram_gb >= 32.0 {
        score += 3;
    } else if ram_gb >= 16.0 {
        score += 2;
    } else if ram_gb >= 8.0 {
        score += 1;
    }

    score += match gpu {
        Some(AdapterKind::Discrete) => 4,
        Some(AdapterKind::Integrated) => 2,
        _ => 0,
    };

    if score <= 3 {
        Tier::Low
    } else if score <= 6 {
        Tier::Mainstream
    } else if score <= 9 {
        Tier::High
    } else {
        Tier::Enthusiast
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_machine_with_no_gpu_is_low() {
        assert_eq!(classify(2, 4.0, None), Tier::Low);
    }

    #[test]
    fn a_typical_laptop_is_mainstream() {
        assert_eq!(classify(8, 16.0, Some(AdapterKind::Integrated)), Tier::Mainstream);
    }

    #[test]
    fn a_strong_desktop_is_high() {
        // 8 threads (+2) + 16 GB (+2) + a discrete GPU (+4) = 8, inside the
        // high band (7..=9) and short of enthusiast (>9).
        assert_eq!(classify(8, 16.0, Some(AdapterKind::Discrete)), Tier::High);
    }

    #[test]
    fn the_top_of_every_band_is_enthusiast() {
        assert_eq!(classify(32, 64.0, Some(AdapterKind::Discrete)), Tier::Enthusiast);
    }

    #[test]
    fn thresholds_are_inclusive_at_their_floor() {
        assert_eq!(classify(4, 8.0, None), Tier::Low);
        assert_eq!(classify(8, 16.0, None), Tier::Mainstream);
    }
}

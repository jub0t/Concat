// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Turning a timeline plus a timestamp into one finished frame.
//!
//! Rendering splits in two, and the split is the important part of this crate:
//!
//! 1. [`plan`] answers "what is on screen at this instant, from where, and how
//!    strongly". It touches no pixels and does no IO, so it is fast, exactly
//!    testable, and identical for the CPU and GPU backends.
//! 2. [`compositor`] takes that plan plus the decoded pixels and blends them.
//!
//! Only step 2 is backend-specific. The CPU compositor is the reference
//! implementation; the GPU one exists to be fast and must match it.

pub mod compositor;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod plan;

pub use compositor::{Compositor, CpuCompositor, Layer, Placement};
#[cfg(feature = "gpu")]
pub use gpu::WgpuCompositor;
pub use plan::{FramePlan, PlannedLayer, plan_frame};

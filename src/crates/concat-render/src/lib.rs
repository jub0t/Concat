// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Turning a timeline plus a timestamp into one finished frame.
//!
//! Rendering splits in two, and the split is the important part of this crate:
//!
//! 1. [`plan`] answers "what is on screen at this instant, from where, and how
//!    strongly". It touches no pixels and does no IO, so it is fast, exactly
//!    testable, and identical for the CPU and GPU backends. The executor
//!    fills the plan out with the decoded pictures and everything the
//!    model has no field for, and a [`FramePlan`] is then the whole
//!    description of the frame.
//! 2. [`compositor`] takes that plan and nothing else, and draws it.
//!
//! Only step 2 is backend-specific. The CPU compositor is the reference
//! implementation; the GPU one exists to be fast and must match it, which
//! the parity suite holds it to.

pub mod compositor;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod kernels;
pub mod metrics;
pub mod plan;
mod transitions;

pub use compositor::{Compositor, CpuCompositor};
#[cfg(feature = "gpu")]
pub use gpu::WgpuCompositor;
pub use metrics::ssim;
pub use plan::{
    Crop, FramePlan, Geometry, PlannedLayer, PlannedTreatment, Shading, Transition, detached_clip,
    plan_frame,
};
/// The wgpu the compositor is built on, for callers that share its device.
#[cfg(feature = "gpu")]
pub use wgpu;

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
//! Step 2 is the GPU's alone: [`WgpuCompositor`] draws every frame, on the
//! platform's software adapter where a machine has no GPU. The CPU
//! compositor that used to be the reference is kept in the tests only, as
//! the oracle the GPU's parity suite checks it against.

pub mod compositor;
pub mod gpu;
#[cfg(test)]
mod kernels;
pub mod metrics;
pub mod plan;
#[cfg(test)]
mod reference;
pub mod scopes;
#[cfg(test)]
mod transitions;

pub use compositor::Compositor;
pub use gpu::WgpuCompositor;
pub use metrics::ssim;
pub use plan::{
    Crop, FramePlan, Geometry, PlannedLayer, PlannedTreatment, Shading, Transition, detached_clip,
    plan_frame,
};
#[cfg(test)]
pub(crate) use reference::CpuCompositor;
pub use scopes::{ScopeData, ScopeKind};
/// The wgpu the compositor is built on, for callers that share its device.
pub use wgpu;

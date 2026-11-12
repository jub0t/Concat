// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

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
//! Step 2 is the GPU's: [`WgpuCompositor`] draws every frame, on the
//! platform's software adapter where a machine has no GPU. The CPU
//! compositor is the oracle the GPU's parity suite checks it against, and
//! the preview's last rung, for when no device will draw.

pub mod compositor;
pub mod gpu;
mod ladder;
pub mod metrics;
pub mod plan;
mod reference;
mod transitions;
pub mod upscale;

pub use compositor::{Compositor, Drawn};
pub use gpu::{ForgedShader, Rung, ShaderForge, WgpuCompositor};
pub use ladder::Ladder;
pub use metrics::ssim;
pub use plan::{
    Backdrop, Crop, FramePlan, Geometry, PlannedLayer, PlannedTreatment, Shading, Transition, Warp,
    detached_clip, plan_frame,
};
pub use reference::CpuCompositor;
/// The wgpu the compositor is built on, for callers that share its device.
pub use wgpu;

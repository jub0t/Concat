// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Blending a plan's layers into one frame.
//!
//! The [`Compositor`] trait is what the export and the monitor draw
//! through, and a [`FramePlan`] is all it takes. The wgpu compositor is the
//! one implementation: a machine without a GPU runs it on the platform's
//! software adapter (WARP, lavapipe). The CPU compositor that was the
//! reference lives on in the tests alone ([`crate::reference`]), as the
//! oracle the GPU is held to while both draw 8-bit, gamma-encoded pictures.

use concat_core::frame::Frame;
use concat_core::shader::TransitionPass;

use crate::plan::FramePlan;

/// Draws a plan into a frame.
pub trait Compositor {
    /// Draws `plan`'s layers bottom-most first over an opaque black
    /// background, every treatment applied over the stack beneath its
    /// track.
    ///
    /// Layers may hang off any edge; anything outside the output is clipped.
    /// The result is always fully opaque - it is what goes to screen or to an
    /// encoder, and neither has anything to show through.
    fn render(&mut self, plan: &FramePlan) -> Frame;

    /// Combines two finished frames with a transition: the outgoing picture
    /// `from` and the incoming one `to`, at the pass's `progress`. The shader
    /// owns the blend. `None` from a compositor that cannot run it: the
    /// caller then shows the fallback dissolve the incoming layer already
    /// carries.
    fn combine(
        &mut self,
        _width: u32,
        _height: u32,
        _time: f32,
        _from: &Frame,
        _to: &Frame,
        _pass: &TransitionPass,
    ) -> Option<Frame> {
        None
    }

    /// Whether the device this draws on has been lost - a reset, a hang, a
    /// driver gone - after which every frame it hands back is black. An
    /// export checks it after each frame and stops with an error rather
    /// than write a file of black.
    fn lost(&self) -> bool {
        false
    }
}

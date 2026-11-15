// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Blending a plan's layers into one frame.
//!
//! The [`Compositor`] trait is what the export and the monitor draw
//! through, and a [`FramePlan`] is all it takes. The wgpu compositor is the
//! one implementation: a machine without a GPU runs it on the platform's
//! software adapter (WARP, lavapipe). The CPU compositor that was the
//! reference lives on in the tests alone ([`crate::reference`]), as the
//! oracle the GPU's plain layers are held to.

use concat_core::frame::{Frame, Yuv420, YuvLayout};
use concat_core::shader::TransitionPass;

use crate::plan::FramePlan;

/// A frame [`Compositor::collect`] hands back: RGBA as
/// [`Compositor::render`] draws it, or the encoder's own YUV where
/// [`Compositor::planar`] asked for it.
pub enum Drawn {
    /// Eight or sixteen bits a channel, RGBA.
    Rgba(Frame),
    /// Eight-bit 4:2:0, converted where it was drawn.
    Yuv(Yuv420),
}

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

    /// Starts drawing `plan` as [`Compositor::render`] would, without
    /// waiting for it: [`Compositor::collect`] hands it back later, oldest
    /// first, so the device draws one frame while the caller reads the
    /// next. False from a compositor that cannot, or cannot take another
    /// yet; the caller then collects, or draws with `render`.
    fn submit(&mut self, _plan: &FramePlan) -> bool {
        false
    }

    /// The oldest frame [`Compositor::submit`] started, once it is drawn;
    /// `None` when none is on its way. A frame the device failed comes back
    /// black, and [`Compositor::lost`] says so, as after `render`.
    fn collect(&mut self) -> Option<Drawn> {
        None
    }

    /// From here on, an SDR frame [`Compositor::submit`] starts comes back
    /// as 4:2:0 YUV in `layout` - BT.709's coefficients, in its range -
    /// converted on the device, so it reads back at a byte and a half a
    /// pixel and the encoder converts nothing. `None` goes back to RGBA. A
    /// frame that cannot be - an HDR file's, one with alpha kept, an odd
    /// side - still comes back RGBA, and a compositor that cannot do it
    /// at all does nothing.
    fn planar(&mut self, _layout: Option<YuvLayout>) {}

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

    /// From here on, frames of an HDR plan come back for an HDR file - the
    /// light on Rec. 2020's primaries, encoded HLG or PQ as the plan's
    /// output says, sixteen bits a channel - rather than rolled off into
    /// eight bits for an SDR one. A compositor that cannot does nothing,
    /// and its frames stay SDR.
    fn deliver_hdr(&mut self, _on: bool) {}

    /// Whether the device this draws on has been lost - a reset, a hang, a
    /// driver gone - after which every frame it hands back is black. An
    /// export checks it after each frame and stops with an error rather
    /// than write a file of black.
    fn lost(&self) -> bool {
        false
    }
}

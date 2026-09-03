// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! A shader pass, as the compositor runs it.
//!
//! An effect on the GPU is a fragment shader over a layer's pixels: the
//! layer goes in as a texture, the pass draws it out again changed, and the
//! result is what gets composited. This is the whole description of one
//! such pass, resolved from a package and a clip's settings by the effect
//! catalogue and carried to the renderer as data - the renderer compiles
//! and caches the pipeline by `key`, and pours `params` into the shader's
//! uniform buffer as it is, because the catalogue already laid the bytes
//! out the way the shader's `Params` struct wants them.
//!
//! It lives here, in the crate every other one can see, so the catalogue
//! that builds it and the compositor that runs it need not know each other.

use std::sync::Arc;

/// One fragment pass over a layer.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderPass {
    /// What to cache the compiled pipeline under: the package's id and
    /// version, so a package that changes its shader gets a new pipeline
    /// and one that only changes its knobs keeps the old.
    pub key: String,
    /// The complete WGSL module: the host's prelude with the package's body,
    /// declaring `fn effect(uv: vec2<f32>) -> vec4<f32>`.
    pub source: Arc<str>,
    /// The `Params` uniform, laid out to the struct's offsets. Sixteen bytes
    /// at least, so an empty struct still has a buffer.
    pub params: Vec<u8>,
    /// How much of the result to keep over the untouched layer, `0..=1`. A
    /// look at half strength is half the look; an effect is always one.
    pub intensity: f32,
}

impl ShaderPass {
    /// The uniform buffer's minimum size: a struct with nothing in it still
    /// needs a binding.
    pub const MIN_PARAMS: usize = 16;
}

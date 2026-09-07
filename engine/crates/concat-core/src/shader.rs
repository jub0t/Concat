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
    /// The package's look-up table, bound as a 3D texture the shader's
    /// `lut()` samples; None binds the identity so the call is harmless.
    pub lut: Option<Arc<Lut>>,
}

impl ShaderPass {
    /// The uniform buffer's minimum size: a struct with nothing in it still
    /// needs a binding.
    pub const MIN_PARAMS: usize = 16;
}

/// A 3D look-up table: `size` texels a side, RGBA8, red fastest, then
/// green, then blue - the order a `.cube` file lists its rows in and the
/// layout a `texture_3d` is uploaded from. A colour is looked up by its
/// own components: the table maps every input colour to an output one.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    /// A hash of the contents, so a renderer can cache the upload.
    pub id: u64,
    /// Texels a side, at least 2.
    pub size: u32,
    /// `size³ × 4` bytes.
    pub rgba: Arc<[u8]>,
}

impl Lut {
    /// A table from `size³` RGB triples in 0..=1, red fastest. Values
    /// outside the range are clamped; a table of the wrong length is None.
    pub fn from_rgb(size: u32, rgb: &[f32]) -> Option<Lut> {
        let texels = (size as usize).checked_pow(3)?;
        if size < 2 || rgb.len() != texels * 3 {
            return None;
        }
        let mut rgba = Vec::with_capacity(texels * 4);
        for triple in rgb.chunks_exact(3) {
            for channel in triple {
                rgba.push((channel.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            rgba.push(255);
        }
        let id = fnv64(&rgba) ^ u64::from(size);
        Some(Lut {
            id,
            size,
            rgba: rgba.into(),
        })
    }

    /// The table that changes nothing: what a pass without one binds.
    pub fn identity(size: u32) -> Lut {
        let size = size.max(2);
        let step = 1.0 / (size - 1) as f32;
        let mut rgb = Vec::with_capacity((size * size * size * 3) as usize);
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    rgb.extend_from_slice(&[r as f32 * step, g as f32 * step, b as f32 * step]);
                }
            }
        }
        Lut::from_rgb(size, &rgb).expect("a square table")
    }

    /// The colour the table maps `rgb` to, trilinearly interpolated - the
    /// same arithmetic the GPU's sampler does, for a CPU that wants it.
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = self.size as usize;
        let last = (n - 1) as f32;
        let at = |c: f32| {
            let x = c.clamp(0.0, 1.0) * last;
            let i = (x.floor() as usize).min(n - 2);
            (i, x - i as f32)
        };
        let (ri, rf) = at(rgb[0]);
        let (gi, gf) = at(rgb[1]);
        let (bi, bf) = at(rgb[2]);
        let texel = |r: usize, g: usize, b: usize| {
            let o = ((b * n + g) * n + r) * 4;
            [
                f32::from(self.rgba[o]) / 255.0,
                f32::from(self.rgba[o + 1]) / 255.0,
                f32::from(self.rgba[o + 2]) / 255.0,
            ]
        };
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ]
        };
        let c00 = lerp(texel(ri, gi, bi), texel(ri + 1, gi, bi), rf);
        let c10 = lerp(texel(ri, gi + 1, bi), texel(ri + 1, gi + 1, bi), rf);
        let c01 = lerp(texel(ri, gi, bi + 1), texel(ri + 1, gi, bi + 1), rf);
        let c11 = lerp(texel(ri, gi + 1, bi + 1), texel(ri + 1, gi + 1, bi + 1), rf);
        lerp(lerp(c00, c10, gf), lerp(c01, c11, gf), bf)
    }
}

/// FNV-1a, so a table's id needs no dependency and is the same on every
/// machine.
fn fnv64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_table_returns_what_it_is_given() {
        let lut = Lut::identity(17);
        for rgb in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.25, 0.5, 0.75], [0.9, 0.1, 0.3]] {
            let out = lut.sample(rgb);
            for (a, b) in out.iter().zip(rgb.iter()) {
                assert!((a - b).abs() < 0.01, "{rgb:?} -> {out:?}");
            }
        }
    }

    #[test]
    fn a_table_of_the_wrong_length_is_refused() {
        assert!(Lut::from_rgb(3, &[0.0; 26 * 3]).is_none());
        assert!(Lut::from_rgb(1, &[0.0; 3]).is_none());
    }
}

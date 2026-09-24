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
//! The parameters are uniforms, never strings: a knob with keys is worth
//! something different each frame, and the catalogue resolves it to the
//! value for the frame and writes the buffer. The same resolved values
//! ride along by name in `values`, for a renderer that runs the package
//! through a kernel of its own rather than the shader - the CPU reference
//! - so both read one resolution of the clip's settings.
//!
//! It lives here, in the crate every other one can see, so the catalogue
//! that builds it and the compositor that runs it need not know each other.

use std::collections::BTreeMap;
use std::sync::Arc;

/// One fragment pass over a layer.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderPass {
    /// The package's id, `author.name`: what a renderer without a shader
    /// stage keys its own kernel for the package by.
    pub package: String,
    /// What to cache the compiled pipeline under: the package's id and
    /// version and a fingerprint of its source, so a package that changes
    /// its shader gets a new pipeline - version bump or not - and one that
    /// only changes its knobs keeps the old.
    pub key: String,
    /// The complete WGSL module: the host's prelude with the package's body,
    /// declaring `fn effect(uv: vec2<f32>) -> vec4<f32>`.
    pub source: Arc<str>,
    /// The `Params` uniform, laid out to the struct's offsets. Sixteen bytes
    /// at least, so an empty struct still has a buffer.
    pub params: Vec<u8>,
    /// The same values by the manifest's keys, every declared parameter
    /// present, resolved for the frame: what `params` was written from.
    pub values: BTreeMap<String, f64>,
    /// How much of the result to keep over the untouched layer, `0..=1`. A
    /// look at half strength is half the look; an effect is always one.
    pub intensity: f32,
    /// The package's look-up table, bound as a 3D texture the shader's
    /// `lut()` samples; None binds the identity so the call is harmless.
    pub lut: Option<Arc<Lut>>,
    /// A title's per-word reveal order, bound as a 2D texture the shader's
    /// `reveal_order()` samples; None binds a map that reveals everything,
    /// so the call is harmless for a pass over anything that is not a
    /// title. See [`RevealMap`].
    pub reveal_map: Option<Arc<RevealMap>>,
}

impl ShaderPass {
    /// The uniform buffer's minimum size: a struct with nothing in it still
    /// needs a binding.
    pub const MIN_PARAMS: usize = 16;
}

/// One two-input transition combine, as the compositor runs it.
///
/// Where a [`ShaderPass`] changes one layer, a transition reads two - the
/// outgoing picture and the incoming one - and a `progress` from 0 to 1, and
/// writes the single picture between them. It is resolved from a transition
/// package and the cut's timing by the effect catalogue and carried to the
/// renderer as data, exactly like a pass: the renderer compiles and caches
/// the pipeline by `key` and pours `params` into the shader's uniform buffer
/// as the catalogue laid them out.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionPass {
    /// What to cache the compiled pipeline under: the package's id and
    /// version, the same rule a [`ShaderPass`] follows.
    pub key: String,
    /// The complete WGSL module: the host's transition prelude with the
    /// package's body, declaring
    /// `fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32>`.
    pub source: Arc<str>,
    /// The `Params` uniform, laid out to the struct's offsets. Sixteen bytes
    /// at least, so an empty struct still has a buffer.
    pub params: Vec<u8>,
    /// How far through the cut this frame is, `0..=1`: 0 is all the outgoing
    /// picture, 1 all the incoming one.
    pub progress: f32,
    /// The package's look-up table, bound like a pass's so the shared
    /// grading helpers work; None binds the identity.
    pub lut: Option<Arc<Lut>>,
    /// The shape a compositor that runs no shaders draws instead: the
    /// FFmpeg `xfade` name the package's manifest declares, which the CPU
    /// reference knows how to draw in plain arithmetic. None, or a name it
    /// does not know, and the compositor declines the combine, leaving the
    /// dissolve the incoming layer already carries.
    pub xfade: Option<String>,
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

/// A title's words, baked into one grayscale map the size of its canvas:
/// each pixel holds the normalized order (0 first, 1 last) of the word
/// painted there, 0 wherever no word was - background is moot since it is
/// transparent regardless, so it is left "already revealed" rather than
/// given a value that would mean something if it were ever visible.
///
/// This is how a per-word reveal effect works without the renderer or the
/// shader knowing what a "word" is: the layout that already happens to
/// paint a title is baked once into a texture, exactly as a colour look-up
/// table bakes a grade, and a shader reads it back with one comparison
/// against `progress`.
#[derive(Clone, Debug, PartialEq)]
pub struct RevealMap {
    /// A hash of the contents, so a renderer can cache the upload.
    pub id: u64,
    /// The canvas width this map was baked at.
    pub width: u32,
    /// The canvas height this map was baked at.
    pub height: u32,
    /// `width * height` bytes, one per pixel, red channel only.
    pub gray: Arc<[u8]>,
}

impl RevealMap {
    /// A map `width` by `height`, `rects` each `(x, y, width, height)` in
    /// canvas pixels and in the order the words should reveal - reading
    /// order is what a caller normally means. A pixel outside every rect
    /// stays revealed from the start; one inside more than one keeps the
    /// later word's order, though words in practice never overlap.
    pub fn from_rects(width: u32, height: u32, rects: &[(i32, i32, u32, u32)]) -> RevealMap {
        let mut gray = vec![0u8; width as usize * height as usize];
        let last = rects.len().saturating_sub(1).max(1) as f32;
        for (index, &(x, y, w, h)) in rects.iter().enumerate() {
            let value = ((index as f32 / last) * 255.0).round() as u8;
            let x0 = x.clamp(0, width as i32) as u32;
            let y0 = y.clamp(0, height as i32) as u32;
            let x1 = (x + w as i32).clamp(0, width as i32) as u32;
            let y1 = (y + h as i32).clamp(0, height as i32) as u32;
            for row in y0..y1 {
                let start = row as usize * width as usize + x0 as usize;
                gray[start..start + (x1 - x0) as usize].fill(value);
            }
        }
        let id = fnv64(&gray) ^ u64::from(width) ^ (u64::from(height) << 32);
        RevealMap {
            id,
            width,
            height,
            gray: gray.into(),
        }
    }

    /// The map that reveals everything from the first frame: what a pass
    /// without one binds, and a two-by-two texture, so the identity is
    /// almost free to keep resident.
    pub fn identity() -> RevealMap {
        RevealMap::from_rects(2, 2, &[])
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
        for rgb in [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.25, 0.5, 0.75],
            [0.9, 0.1, 0.3],
        ] {
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

    #[test]
    fn the_identity_reveal_map_reveals_everywhere() {
        let map = RevealMap::identity();
        assert!(map.gray.iter().all(|&g| g == 0));
    }

    #[test]
    fn word_rects_are_ordered_zero_to_full_and_outside_stays_revealed() {
        let map = RevealMap::from_rects(10, 10, &[(0, 0, 2, 2), (5, 0, 2, 2), (0, 5, 2, 2)]);
        assert_eq!(map.gray[0], 0);
        assert_eq!(map.gray[5], 128);
        assert_eq!(map.gray[5 * 10], 255);
        // A pixel no rect covers is left at the "already revealed" value.
        assert_eq!(map.gray[9 * 10 + 9], 0);
    }

    #[test]
    fn a_single_word_reveals_at_the_very_start() {
        let map = RevealMap::from_rects(4, 4, &[(0, 0, 2, 2)]);
        assert_eq!(map.gray[0], 0);
    }
}

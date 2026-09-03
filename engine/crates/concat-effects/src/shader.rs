// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! A package's shader: what it declares, checked and laid out at load.
//!
//! A WGSL package writes two things and nothing else: a `Params` struct
//! whose fields are its knobs, and `fn effect(uv: vec2<f32>) -> vec4<f32>`,
//! the colour it wants at a point of the layer. Everything around that -
//! the bindings, the frame's size and time, the vertex stage, the mixing of
//! the result back over the untouched layer by intensity - is the host's,
//! and is stitched on here so every package shares one contract and no
//! package can bind things differently.
//!
//! The stitched module is parsed and validated when the package loads, the
//! same way a chain template is, so a broken shader is a load error and not
//! a black frame. Its `Params` struct is read back through naga for the
//! offset of every field, which is how a clip's settings become the bytes
//! of a uniform buffer without a package having to say anything about
//! layout.

use std::collections::BTreeMap;
use std::sync::Arc;

use concat_core::ShaderPass;

use crate::manifest::{Manifest, Param, ParamType};

/// What every package's shader can see. Group 0 is the layer, group 1 the
/// host's frame block and the package's own parameters.
pub const PRELUDE: &str = r#"// ── the host's half of the contract; see concat-effects/src/shader.rs ──
struct Frame {
    /// The layer's size in pixels.
    size: vec2<f32>,
    /// Seconds into the timeline.
    time: f32,
    /// How much of the effect to keep over the untouched layer.
    intensity: f32,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(1) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(1) var<uniform> params: Params;

/// The layer's colour at `uv`, straight alpha.
fn sample(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(source, source_sampler, uv);
}

/// One pixel, as a fraction of the layer.
fn texel() -> vec2<f32> {
    return vec2<f32>(1.0, 1.0) / frame.size;
}

/// Luminance, Rec. 709.
fn luma(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
}

/// A hash in 0..1 from a point and a seed, for grain and dither.
fn hash(p: vec2<f32>, seed: f32) -> f32 {
    let q = vec3<f32>(p, seed);
    return fract(sin(dot(q, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}
"#;

/// The stages the host draws with: a full-screen triangle, and a fragment
/// that mixes the package's colour over the untouched layer by intensity.
pub const POSTLUDE: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    // One triangle over the whole target; the clip trims it to the square.
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    var out: VsOut;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let base = sample(in.uv);
    let treated = effect(in.uv);
    return mix(base, treated, clamp(frame.intensity, 0.0, 1.0));
}
"#;

/// Where one parameter lands in the uniform buffer.
#[derive(Clone, Debug, PartialEq)]
struct Slot {
    key: String,
    offset: usize,
    kind: ParamType,
}

/// A package's shader, stitched, checked and laid out.
#[derive(Clone, Debug)]
pub struct Shader {
    key: String,
    source: Arc<str>,
    slots: Vec<Slot>,
    span: usize,
}

impl Shader {
    /// Stitches `body` into the host's contract, checks it, and reads the
    /// `Params` struct for where each of the manifest's parameters lands.
    /// Every declared parameter must be a field of the struct; a field the
    /// manifest does not declare is allowed and stays zero.
    pub fn compile(manifest: &Manifest, body: &str) -> Result<Shader, String> {
        let declares_params = body
            .split("struct")
            .skip(1)
            .any(|rest| rest.trim_start().starts_with("Params"));
        if !body.contains("fn effect") {
            return Err(
                "the shader declares no `fn effect(uv: vec2<f32>) -> vec4<f32>`".to_owned(),
            );
        }
        let mut source = String::with_capacity(PRELUDE.len() + body.len() + POSTLUDE.len() + 64);
        if !declares_params {
            // A package with no knobs still has to bind something.
            source.push_str("struct Params { _unused: f32 }\n");
        }
        source.push_str(body);
        source.push('\n');
        source.push_str(PRELUDE);
        source.push_str(POSTLUDE);

        let module =
            naga::front::wgsl::parse_str(&source).map_err(|error| error.emit_to_string(&source))?;
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .map_err(|error| error.emit_to_string(&source))?;
        if !module
            .functions
            .iter()
            .any(|(_, function)| function.name.as_deref() == Some("effect"))
        {
            return Err(
                "the shader declares no `fn effect(uv: vec2<f32>) -> vec4<f32>`".to_owned(),
            );
        }

        let (members, span) = module
            .types
            .iter()
            .find_map(|(_, ty)| match (&ty.name, &ty.inner) {
                (Some(name), naga::TypeInner::Struct { members, span }) if name == "Params" => {
                    Some((members.clone(), *span as usize))
                }
                _ => None,
            })
            .ok_or_else(|| "the shader declares no `struct Params`".to_owned())?;

        let mut slots = Vec::new();
        for param in &manifest.params {
            let member = members
                .iter()
                .find(|member| member.name.as_deref() == Some(param.key.as_str()))
                .ok_or_else(|| format!("`Params` has no field `{}`", param.key))?;
            let inner = &module.types[member.ty].inner;
            let wanted = match param.kind {
                ParamType::Point => 2,
                ParamType::Color => 4,
                _ => 1,
            };
            let width = match inner {
                naga::TypeInner::Scalar(scalar) if is_f32(scalar) => 1,
                naga::TypeInner::Vector { size, scalar } if is_f32(scalar) => *size as usize,
                _ => 0,
            };
            if width != wanted {
                return Err(format!(
                    "`Params.{}` must be {}",
                    param.key,
                    match wanted {
                        2 => "a vec2<f32>",
                        4 => "a vec4<f32>",
                        _ => "an f32",
                    }
                ));
            }
            slots.push(Slot {
                key: param.key.clone(),
                offset: member.offset as usize,
                kind: param.kind,
            });
        }

        Ok(Shader {
            key: format!("{}@{}", manifest.effect.id, manifest.effect.version),
            source: Arc::from(source),
            slots,
            span: span.max(ShaderPass::MIN_PARAMS).div_ceil(16) * 16,
        })
    }

    /// The stitched module.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The `Params` buffer for these resolved values, laid out to the
    /// struct. Every declared parameter is present in `values` by the time
    /// the catalogue calls this; anything missing reads as zero.
    pub fn params_bytes(&self, values: &BTreeMap<String, f64>, params: &[Param]) -> Vec<u8> {
        let mut bytes = vec![0u8; self.span];
        let mut put = |offset: usize, value: f64| {
            let at = offset..offset + 4;
            if at.end <= bytes.len() {
                bytes[at].copy_from_slice(&(value as f32).to_le_bytes());
            }
        };
        for slot in &self.slots {
            match slot.kind {
                ParamType::Point => {
                    put(
                        slot.offset,
                        values
                            .get(&format!("{}.x", slot.key))
                            .copied()
                            .unwrap_or(0.5),
                    );
                    put(
                        slot.offset + 4,
                        values
                            .get(&format!("{}.y", slot.key))
                            .copied()
                            .unwrap_or(0.5),
                    );
                }
                ParamType::Color => {
                    // Packed RGBA in one number, as the document stores it.
                    let packed = values.get(&slot.key).copied().unwrap_or(0.0).max(0.0) as u32;
                    for (index, shift) in [24u32, 16, 8, 0].into_iter().enumerate() {
                        put(
                            slot.offset + index * 4,
                            f64::from((packed >> shift) & 0xff) / 255.0,
                        );
                    }
                }
                _ => {
                    let fallback = params
                        .iter()
                        .find(|param| param.key == slot.key)
                        .map(|param| param.default)
                        .unwrap_or(0.0);
                    put(
                        slot.offset,
                        values.get(&slot.key).copied().unwrap_or(fallback),
                    );
                }
            }
        }
        bytes
    }

    /// A pass over a layer with these values.
    pub fn pass(
        &self,
        values: &BTreeMap<String, f64>,
        params: &[Param],
        intensity: f32,
    ) -> ShaderPass {
        ShaderPass {
            key: self.key.clone(),
            source: Arc::clone(&self.source),
            params: self.params_bytes(values, params),
            intensity,
        }
    }
}

fn is_f32(scalar: &naga::Scalar) -> bool {
    scalar.kind == naga::ScalarKind::Float && scalar.width == 4
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(params: &str) -> Manifest {
        Manifest::parse(&format!(
            r#"
[effect]
id = "test.thing"
name = "Thing"
kind = "effect"
{params}
[wgsl]
entry = "effect.wgsl"
"#
        ))
        .expect("a valid manifest")
    }

    #[test]
    fn a_body_is_stitched_checked_and_laid_out() {
        let manifest = manifest(
            r#"
[[param]]
key = "amount"
label = "Amount"
min = 0
max = 1
default = 0.25

[[param]]
key = "radius"
label = "Radius"
min = 0
max = 10
default = 2
"#,
        );
        let shader = Shader::compile(
            &manifest,
            r#"
struct Params { radius: f32, amount: f32 }
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    return vec4<f32>(c.rgb * params.amount + params.radius * 0.0, c.a);
}
"#,
        )
        .expect("compiles");
        assert_eq!(shader.key, "test.thing@1");
        assert!(shader.source().contains("fn fs_main"));
        // radius first at 0, amount at 4; the buffer padded to sixteen.
        let mut values = BTreeMap::new();
        values.insert("amount".to_owned(), 0.5);
        let bytes = shader.params_bytes(&values, &manifest.params);
        assert_eq!(bytes.len(), 16);
        assert_eq!(f32::from_le_bytes(bytes[0..4].try_into().unwrap()), 2.0);
        assert_eq!(f32::from_le_bytes(bytes[4..8].try_into().unwrap()), 0.5);
    }

    #[test]
    fn a_package_with_no_knobs_still_binds() {
        let manifest = manifest("");
        let shader = Shader::compile(
            &manifest,
            "fn effect(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }",
        )
        .expect("compiles");
        assert_eq!(shader.params_bytes(&BTreeMap::new(), &[]).len(), 16);
    }

    #[test]
    fn a_missing_field_or_a_broken_body_is_refused() {
        let manifest = manifest(
            r#"
[[param]]
key = "amount"
label = "Amount"
"#,
        );
        let missing = Shader::compile(
            &manifest,
            "struct Params { other: f32 }\nfn effect(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }",
        );
        assert!(missing.unwrap_err().contains("no field `amount`"));
        let broken = Shader::compile(
            &manifest,
            "struct Params { amount: f32 }\nfn effect(uv: vec2<f32>) -> vec4<f32> { return nonsense; }",
        );
        assert!(broken.is_err());
        let no_effect = Shader::compile(&manifest, "struct Params { amount: f32 }");
        assert!(no_effect.unwrap_err().contains("fn effect"));
    }
}
